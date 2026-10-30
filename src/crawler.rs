use crate::{settings::Settings, state::AppState};
use anyhow::{Context as _, Result};
use rusqlite::{params, OptionalExtension};
use scraper::{Html, Selector};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use url::Url;

const AGENT: &str = "SSSE";
const USER_AGENT: &str = "SSSE/0.1 (self-hosted search crawler)";

#[derive(Clone)]
struct Task {
    id: i64,
    url: String,
    depth: u32,
    sitemap: bool,
    scope: String,
    parent_depth: Option<u32>,
}

#[derive(Default)]
struct Work {
    fifo: VecDeque<Task>,
    active: HashSet<i64>,
}

struct Crawl {
    state: AppState,
    settings: Settings,
    client: reqwest::Client,
    timers: AsyncMutex<HashMap<String, Instant>>,
    origins: AsyncMutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    banned_domains: Vec<String>,
    banned_words: Vec<(String, String)>,
    work: Arc<Mutex<Work>>,
    sitemap_origins: Mutex<HashMap<String, (u32, Vec<String>)>>,
}

pub async fn run(state: AppState, settings: Settings, cancel: CancellationToken) -> Result<bool> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .build()?;
    run_with_client(state, settings, cancel, client).await
}

async fn run_with_client(
    state: AppState,
    settings: Settings,
    cancel: CancellationToken,
    client: reqwest::Client,
) -> Result<bool> {
    let conn = state.db();
    crate::settings::set(
        &conn,
        "last_attempt",
        &chrono::Utc::now().timestamp().to_string(),
    );
    let banned_domains = conn
        .prepare("SELECT domain FROM banned_domains")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    let banned_words = conn
        .prepare("SELECT word,mode FROM banned_words")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let seeds = conn
        .prepare("SELECT url FROM seed_urls ORDER BY url")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    let crawl = Arc::new(Crawl {
        state: state.clone(),
        settings: settings.clone(),
        client,
        timers: AsyncMutex::new(HashMap::new()),
        origins: AsyncMutex::new(HashMap::new()),
        banned_domains,
        banned_words,
        work: Arc::new(Mutex::new(Work::default())),
        sitemap_origins: Mutex::new(HashMap::new()),
    });
    for seed in seeds {
        crawl.enqueue(&seed, 0)?;
    }
    drop(conn);
    let count = settings.workers();
    let work = crawl.work.clone();
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..count {
        let crawl = crawl.clone();
        let work = work.clone();
        workers.spawn(async move {
            loop {
                let task = {
                    let mut work = work.lock().unwrap();
                    let task = work.fifo.pop_front();
                    if let Some(task) = &task {
                        work.active.insert(task.id);
                    }
                    task
                };
                if let Some(task) = task {
                    crawl.state.inner.lock().unwrap().active_workers += 1;
                    tokio::time::sleep(Duration::from_millis(crawl.settings.worker_delay_ms)).await;
                    let result = crawl.process(&task).await;
                    crawl.state.inner.lock().unwrap().active_workers -= 1;
                    result?;
                    let mut work = work.lock().unwrap();
                    crawl
                        .state
                        .db()
                        .execute("DELETE FROM crawl_queue WHERE id=?1", [task.id])?;
                    work.active.remove(&task.id);
                } else {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        });
    }
    let mut last_refill = Instant::now()
        .checked_sub(Duration::from_secs(86400))
        .unwrap_or_else(Instant::now);
    let result: Result<bool> = async {
        loop {
            if cancel.is_cancelled() { return Ok(false); }
            if let Some(result)=workers.try_join_next() { result.context("crawler worker panicked")??; anyhow::bail!("crawler worker exited unexpectedly"); }
            if last_refill.elapsed()>=Duration::from_millis(settings.fifo_refill_ms) {
                let mut work=work.lock().unwrap();
                if work.fifo.len()<count {
                    let mut conn=state.db(); let tx=conn.transaction()?;
                    let tasks=tx.prepare("SELECT id,url,depth,kind,scope,parent_depth FROM crawl_queue WHERE assigned=0 ORDER BY id LIMIT ?1")?
                        .query_map([(count*2-work.fifo.len()) as i64],|r|Ok(Task{id:r.get(0)?,url:r.get(1)?,depth:r.get(2)?,sitemap:r.get::<_,String>(3)?=="sitemap",scope:r.get(4)?,parent_depth:r.get(5)?}))?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    for task in tasks { tx.execute("UPDATE crawl_queue SET assigned=1 WHERE id=?1",[task.id])?; work.fifo.push_back(task); }
                    tx.commit()?;
                }
                last_refill=Instant::now();
            }
            let pending:i64=state.db().query_row("SELECT count(*) FROM crawl_queue",[],|r|r.get(0))?;
            if pending==0 && work.lock().unwrap().active.is_empty() { return Ok(true); }
            tokio::select! { _=cancel.cancelled()=>return Ok(false), _=tokio::time::sleep(Duration::from_millis(20))=>{} }
        }
    }.await;
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    if result? && !cancel.is_cancelled() {
        publish(&state, Some(&cancel))
    } else {
        Ok(false)
    }
}

impl Crawl {
    fn banned_domain(&self, host: &str) -> bool {
        self.banned_domains
            .iter()
            .any(|d| host == d || host.ends_with(&format!(".{d}")))
    }
    fn banned(&self, text: &str) -> bool {
        self.banned_words
            .iter()
            .any(|(word, mode)| banned_word_matches(word, mode, text))
    }
    fn refuse(&self, url: &str, reason: &str, status: Option<u16>) -> Result<()> {
        let domain = Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(registrable));
        self.state.db().execute(
            "INSERT INTO staging_refused_urls(url,domain,reason,status,ts) VALUES(?1,?2,?3,?4,?5)",
            params![url, domain, reason, status, chrono::Utc::now().timestamp()],
        )?;
        self.state.inner.lock().unwrap().refused += 1;
        Ok(())
    }
    fn eligible(&self, url: &Url) -> Option<&'static str> {
        if url.scheme() == "http" {
            Some("http_url")
        } else if url.scheme() != "https" {
            Some("unsupported_protocol")
        } else if !matches!(url.host(), Some(url::Host::Domain(_))) {
            Some("ip_literal")
        } else if !url.username().is_empty() || url.password().is_some() {
            Some("url_credentials")
        } else if self.banned_domain(url.host_str().unwrap_or("")) {
            Some("banned_domain")
        } else if self.banned(url.as_str()) {
            Some("banned_word")
        } else {
            None
        }
    }
    fn enqueue(&self, raw: &str, depth: u32) -> Result<()> {
        let scope = Url::parse(raw)
            .ok()
            .and_then(|u| u.host_str().map(registrable))
            .unwrap_or_default();
        self.enqueue_task(raw, depth, false, &scope, None)
    }
    fn enqueue_discovered(&self, raw: &str, parent_depth: u32, scope: &str) -> Result<()> {
        if Url::parse(raw)
            .ok()
            .is_some_and(|url| url.path().ends_with(".xml") || url.path().ends_with(".xml.gz"))
        {
            return self.enqueue_task(raw, parent_depth, true, scope, None);
        }
        let depth = Url::parse(raw)
            .ok()
            .map_or(0, |url| child_depth(scope, parent_depth, &url));
        self.enqueue_task(raw, depth, false, scope, Some(parent_depth))
    }
    fn enqueue_task(
        &self,
        raw: &str,
        depth: u32,
        sitemap: bool,
        scope: &str,
        parent_depth: Option<u32>,
    ) -> Result<()> {
        let Some(normalized) = normalize_url(raw) else {
            return self.refuse(raw, "invalid_url", None);
        };
        let url = Url::parse(&normalized)?;
        if let Some(reason) = self.eligible(&url) {
            return self.refuse(&normalized, reason, None);
        }
        if !sitemap && self.settings.max_depth != 0 && depth > self.settings.max_depth {
            return self.refuse(&normalized, "max_depth", None);
        }
        let mut work = self.work.lock().unwrap();
        let mut conn = self.state.db();
        let tx = conn.transaction()?;
        let mut refused = false;
        if tx.execute(
            "INSERT OR IGNORE INTO crawl_seen_urls(url) VALUES(?1)",
            [&normalized],
        )? == 1
        {
            let cached = tx
                .query_row(
                    "SELECT rules,fetched_at FROM crawl_robots WHERE origin=?1",
                    [url.origin().ascii_serialization()],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                )
                .optional()?;
            if cached.is_some_and(|(rules, ts)| {
                chrono::Utc::now().timestamp() - ts < 86400 && !robots_url_allowed(&rules, &url)
            }) {
                tx.execute("INSERT INTO staging_refused_urls(url,domain,reason,ts) VALUES(?1,?2,'robots',?3)",params![normalized,registrable(url.host_str().unwrap()),chrono::Utc::now().timestamp()])?;
                refused = true;
            } else {
                tx.execute("INSERT INTO crawl_queue(url,depth,assigned,created_at,kind,scope,origin,parent_depth) VALUES(?1,?2,0,?3,?4,?5,?6,?7)",params![normalized,depth,chrono::Utc::now().timestamp(),if sitemap {"sitemap"} else {"page"},scope,url.origin().ascii_serialization(),parent_depth])?;
            }
        } else {
            // A shorter pending discovery must not depend on worker/refill order.
            let changed=tx.execute("UPDATE crawl_queue SET depth=?1,scope=?3,parent_depth=?4 WHERE url=?2 AND depth>?1 AND ((kind='page' AND ?5=0) OR (kind='sitemap' AND ?5=1 AND scope=?3))",params![depth,normalized,scope,parent_depth,sitemap])?;
            if changed > 0 {
                for task in &mut work.fifo {
                    if task.url == normalized {
                        task.depth = depth;
                        task.scope = scope.to_owned();
                        task.parent_depth = parent_depth;
                    }
                }
            }
        }
        tx.commit()?;
        if refused {
            self.state.inner.lock().unwrap().refused += 1;
        }
        Ok(())
    }

    fn discover_sitemaps(&self, task: &Task, url: &Url, html_maps: &[String]) -> Result<()> {
        let origin = url.origin().ascii_serialization();
        let scope = registrable(url.host_str().unwrap());
        let rules: String = self.state.db().query_row(
            "SELECT rules FROM crawl_robots WHERE origin=?1",
            [&origin],
            |r| r.get(0),
        )?;
        self.discover_origin_sitemaps(&origin, &rules, task.depth)?;
        for map in html_maps {
            self.enqueue_task(map, task.depth, true, &scope, None)?;
        }
        Ok(())
    }

    fn discover_origin_sitemaps(&self, origin: &str, rules: &str, depth: u32) -> Result<()> {
        let (_, mut maps) = robots_extensions(rules);
        if maps.is_empty() {
            maps = vec![
                format!("{origin}/sitemap.xml"),
                format!("{origin}/sitemap_index.xml"),
            ];
        }
        maps.sort();
        maps.dedup();
        let depth = {
            let mut origins = self.sitemap_origins.lock().unwrap();
            let previous = origins.get(origin);
            if previous
                .is_some_and(|(old_depth, old_maps)| *old_depth <= depth && old_maps == &maps)
            {
                return Ok(());
            }
            let depth = previous.map_or(depth, |(old_depth, _)| depth.min(*old_depth));
            origins.insert(origin.to_owned(), (depth, maps.clone()));
            depth
        };
        let base = Url::parse(&format!("{origin}/"))?;
        let scope = registrable(base.host_str().unwrap());
        for map in maps {
            if let Some(map) = resolve(&base, &map) {
                self.enqueue_task(&map, depth, true, &scope, None)?;
            }
        }
        Ok(())
    }

    fn purge_robots(&self, origin: &str, rules: &str) -> Result<()> {
        // Refill, leasing, and purging share the same lock/order. Active leases
        // stay in SQLite until their worker rechecks the current rules and acks.
        let mut work = self.work.lock().unwrap();
        let mut conn = self.state.db();
        let tx = conn.transaction()?;
        let tasks = tx
            .prepare("SELECT id,url FROM crawl_queue WHERE origin=?1")?
            .query_map([origin], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut removed = HashSet::new();
        for (id, raw) in tasks {
            if work.active.contains(&id) {
                continue;
            }
            let Ok(url) = Url::parse(&raw) else { continue };
            if url.origin().ascii_serialization() != origin || robots_url_allowed(rules, &url) {
                continue;
            }
            tx.execute("DELETE FROM crawl_queue WHERE id=?1", [id])?;
            tx.execute(
                "INSERT INTO staging_refused_urls(url,domain,reason,ts) VALUES(?1,?2,'robots',?3)",
                params![
                    raw,
                    registrable(url.host_str().unwrap()),
                    chrono::Utc::now().timestamp()
                ],
            )?;
            removed.insert(id);
        }
        tx.commit()?;
        work.fifo.retain(|task| !removed.contains(&task.id));
        self.state.inner.lock().unwrap().refused += removed.len() as u64;
        Ok(())
    }
    async fn reserve(&self, url: &Url, website_delay: f64) -> Instant {
        let reg = registrable(url.host_str().unwrap_or(""));
        let delay = Duration::from_millis(
            self.settings
                .worker_delay_ms
                .saturating_add(self.settings.domain_delay_ms),
        )
        .saturating_add(Duration::from_secs_f64(website_delay.max(0.0)));
        loop {
            let wait = {
                let mut timers = self.timers.lock().await;
                let now = Instant::now();
                let next = timers.get(&reg).copied().unwrap_or(now);
                if next <= now {
                    timers.insert(
                        reg.clone(),
                        now.checked_add(delay)
                            .unwrap_or_else(|| now + Duration::from_secs(100 * 365 * 86400)),
                    );
                    return now;
                }
                next.saturating_duration_since(now)
            };
            // Recheck after waking: late workers must not burst through expired
            // future reservations together when the executor was busy.
            tokio::time::sleep(wait).await;
        }
    }
    async fn robots(&self, url: &Url) -> Result<(bool, f64)> {
        let origin = url.origin().ascii_serialization();
        let lock = self
            .origins
            .lock()
            .await
            .entry(origin.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = lock.lock().await;
        let cached = self
            .state
            .db()
            .query_row(
                "SELECT rules,crawl_delay,fetched_at FROM crawl_robots WHERE origin=?1",
                [&origin],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, f64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let (rules, delay) = match cached {
            Some((rules, delay, ts)) if chrono::Utc::now().timestamp() - ts < 86400 => {
                (rules, delay)
            }
            _ => self.fetch_robots(&origin).await?,
        };
        Ok((robots_url_allowed(&rules, url), delay))
    }
    async fn fetch_robots(&self, origin: &str) -> Result<(String, f64)> {
        let initial = format!("{origin}/robots.txt");
        let mut target = Url::parse(&initial)?;
        let mut status = None;
        let mut rules = "User-agent: *\nDisallow: /".to_owned();
        for hop in 0..=5 {
            // Robots redirects have their own HTTP exception, but never permit IP literals,
            // credentials, unsupported schemes or administrator-banned destinations.
            if !matches!(target.scheme(), "http" | "https")
                || !matches!(target.host(), Some(url::Host::Domain(_)))
                || !target.username().is_empty()
                || target.password().is_some()
                || self.banned_domain(target.host_str().unwrap_or(""))
            {
                break;
            }
            self.reserve(&target, 0.0).await;
            let response = match self.client.get(target.clone()).send().await {
                Ok(r) => r,
                Err(_) => break,
            };
            status = Some(response.status().as_u16());
            if response.status().is_redirection() {
                if hop == 5 {
                    break;
                }
                let next = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| target.join(v).ok());
                if let Some(next) = next {
                    target = next;
                    continue;
                } else {
                    break;
                }
            }
            if response.status().is_client_error() {
                rules.clear();
                break;
            }
            if !response.status().is_success() {
                break;
            }
            let limit = if self.settings.max_html_bytes == 0 {
                0
            } else {
                self.settings.max_html_bytes.max(512 * 1024)
            };
            if let Ok(bytes) = read_body(response, limit).await {
                rules = String::from_utf8_lossy(&bytes).into_owned();
            }
            break;
        }
        let (delay, _) = robots_extensions(&rules);
        {
            let mut conn = self.state.db();
            let tx = conn.transaction()?;
            for table in ["crawl_robots", "staging_robots"] {
                tx.execute(&format!("INSERT OR REPLACE INTO {table}(origin,url,status,rules,crawl_delay,fetched_at) VALUES(?1,?2,?3,?4,?5,?6)"),params![origin,initial,status,rules,delay,chrono::Utc::now().timestamp()])?;
            }
            tx.commit()?;
        }
        self.purge_robots(origin, &rules)?;
        let previous_depth = self
            .sitemap_origins
            .lock()
            .unwrap()
            .get(origin)
            .map(|(depth, _)| *depth);
        if let Some(depth) = previous_depth {
            self.discover_origin_sitemaps(origin, &rules, depth)?;
        }
        Ok((rules, delay))
    }
    async fn process(&self, task: &Task) -> Result<()> {
        let mut current = task.clone();
        if let Some((depth, scope, parent_depth)) = self
            .state
            .db()
            .query_row(
                "SELECT depth,scope,parent_depth FROM crawl_queue WHERE id=?1",
                [task.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
        {
            current.depth = depth;
            current.scope = scope;
            current.parent_depth = parent_depth;
        }
        let task = &current;
        self.state.inner.lock().unwrap().processed += 1;
        let url = Url::parse(&task.url)?;
        if let Some(reason) = self.eligible(&url) {
            return self.refuse(url.as_str(), reason, None);
        }
        loop {
            let (allowed, delay) = self.robots(&url).await?;
            if !allowed {
                return self.refuse(url.as_str(), "robots", None);
            }
            self.reserve(&url, delay).await;
            let (allowed, current_delay) = self.robots(&url).await?;
            if !allowed {
                return self.refuse(url.as_str(), "robots", None);
            }
            if current_delay <= delay {
                break;
            }
        }
        let started = Instant::now();
        let response = match self.client.get(url.clone()).send().await {
            Ok(r) => r,
            Err(e) => {
                let kind = if e.is_timeout() {
                    "timeout"
                } else {
                    "dns_or_connection"
                };
                self.state.db().execute("INSERT OR REPLACE INTO staging_http_responses(url,status,error) VALUES(?1,NULL,?2)",params![url.as_str(),kind])?;
                return self.refuse(url.as_str(), kind, None);
            }
        };
        let status = response.status().as_u16();
        self.state.db().execute("INSERT OR REPLACE INTO staging_http_responses(url,status,error,duration_ms) VALUES(?1,?2,NULL,?3)",params![url.as_str(),status,started.elapsed().as_millis() as i64])?;
        if !response.status().is_success() {
            return self.refuse(
                url.as_str(),
                if response.status().is_redirection() {
                    "redirect"
                } else {
                    "http_error"
                },
                Some(status),
            );
        }
        let headers = response.headers();
        if headers
            .get("content-disposition")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.to_ascii_lowercase().contains("attachment"))
        {
            return self.refuse(url.as_str(), "attachment", Some(status));
        }
        let content_type = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let xml = task.sitemap
            || (content_type.contains("xml") && !content_type.contains("xhtml"))
            || url.path().ends_with(".xml")
            || url.path().ends_with(".xml.gz");
        let (noindex, nofollow) = headers
            .get_all("x-robots-tag")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .map(robot_directives)
            .fold((false, false), |(i, f), (next_i, next_f)| {
                (i || next_i, f || next_f)
            });
        if noindex && !xml {
            return self.refuse(url.as_str(), "noindex", Some(status));
        }
        if xml && nofollow {
            return self.refuse(url.as_str(), "nofollow", Some(status));
        }
        if content_type.starts_with("image/") {
            self.state.db().execute(
                "INSERT OR IGNORE INTO staging_images(url,domain,title) VALUES(?1,?2,?3)",
                params![
                    url.as_str(),
                    registrable(url.host_str().unwrap()),
                    url.path()
                ],
            )?;
            return Ok(());
        }
        if !xml
            && !content_type.contains("text/html")
            && !content_type.contains("application/xhtml")
        {
            return self.refuse(url.as_str(), "unsupported_file_type", Some(status));
        }
        let body = match read_body(response, self.settings.max_html_bytes).await {
            Ok(b) => b,
            Err(BodyError::Oversized) => {
                return self.refuse(url.as_str(), "oversized_page", Some(status))
            }
            Err(BodyError::Network) => {
                return self.refuse(url.as_str(), "connection_failure", Some(status))
            }
        };
        self.state.db().execute(
            "UPDATE staging_http_responses SET duration_ms=?1 WHERE url=?2",
            params![started.elapsed().as_millis() as i64, url.as_str()],
        )?;
        if xml {
            let map = match crate::sitemap::parse(&body, self.settings.max_html_bytes) {
                Ok(map) => map,
                Err(crate::sitemap::Error::Oversized) => {
                    return self.refuse(url.as_str(), "oversized_page", Some(status))
                }
                Err(crate::sitemap::Error::Invalid) => {
                    return self.refuse(url.as_str(), "invalid_sitemap", Some(status))
                }
            };
            let parent = task.parent_depth.unwrap_or(task.depth);
            for loc in map.urls {
                if map.index {
                    self.enqueue_task(&loc, parent, true, &task.scope, None)?;
                } else {
                    self.enqueue_discovered(&loc, parent, &task.scope)?;
                }
            }
            return Ok(());
        }
        let html = String::from_utf8_lossy(&body);
        let mut extracted = {
            let document = Html::parse_document(&html);
            extract(&document, &url, noindex, nofollow)
        };
        if extracted.noindex {
            return self.refuse(url.as_str(), "noindex", Some(status));
        }
        if self.settings.max_text_bytes != 0 && extracted.text.len() > self.settings.max_text_bytes
        {
            return self.refuse(url.as_str(), "oversized_text", Some(status));
        }
        if self.banned(&format!(
            "{} {} {} {} {}",
            url, extracted.title, extracted.description, extracted.text, extracted.metadata
        )) {
            return self.refuse(url.as_str(), "banned_word", Some(status));
        }
        let reg = registrable(url.host_str().unwrap());
        let canonical = extracted
            .canonical
            .take()
            .and_then(|u| Url::parse(&u).ok())
            .filter(|target| target.origin() == url.origin() && self.eligible(target).is_none());
        let index_url = url.as_str();
        if !extracted.nofollow {
            self.discover_sitemaps(task, &url, &extracted.sitemaps)?;
            if let Some(target) = &canonical {
                if target != &url {
                    self.enqueue_discovered(target.as_str(), task.depth, &reg)?;
                }
            }
        }
        let media: Vec<_> = extracted
            .images
            .iter()
            .chain(extracted.videos.iter())
            .cloned()
            .collect();
        let mut accepted = Vec::new();
        for mut item in media {
            let Some(u) = normalize_url(&item.url).and_then(|s| Url::parse(&s).ok()) else {
                continue;
            };
            if self.eligible(&u).is_some() || self.banned(&item.label) {
                self.refuse(&item.url, "media_restriction", None)?;
                continue;
            }
            if !self.robots(&u).await?.0 {
                self.refuse(&item.url, "robots", None)?;
                continue;
            }
            if item.video
                && [".mp4", ".webm", ".mov", ".mkv", ".avi", ".m3u8"]
                    .iter()
                    .any(|ext| u.path().to_ascii_lowercase().ends_with(ext))
            {
                self.refuse(&item.url, "unsupported_file_type", None)?;
                continue;
            }
            if let Some(thumbnail) = item.thumbnail.as_ref().and_then(|s| Url::parse(s).ok()) {
                if self.eligible(&thumbnail).is_some() || !self.robots(&thumbnail).await?.0 {
                    item.thumbnail = None;
                }
            }
            accepted.push((u, item));
        }
        {
            let mut conn = self.state.db();
            let tx = conn.transaction()?;
            if let Some(target) = canonical {
                tx.execute(
                    "INSERT OR REPLACE INTO crawl_canonicals(source,target) VALUES(?1,?2)",
                    params![url.as_str(), target.as_str()],
                )?;
            }
            tx.execute("INSERT INTO staging_pages(url,domain,title,description,text,language,published_at,crawled_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(url) DO NOTHING",params![index_url,reg,extracted.title,extracted.description,extracted.text,extracted.language,extracted.published,chrono::Utc::now().timestamp()])?;
            if extracted.news {
                tx.execute("INSERT OR IGNORE INTO staging_news(url,title,domain,published_at,description) VALUES(?1,?2,?3,?4,?5)",params![index_url,extracted.title,reg,extracted.published,format!("{} {}",extracted.description,extracted.text)])?;
            }
            for (u, item) in accepted {
                let domain = registrable(u.host_str().unwrap());
                if item.video {
                    tx.execute("INSERT OR IGNORE INTO staging_videos(url,title,domain,thumbnail_url,published_at,description) VALUES(?1,?2,?3,?4,?5,?6)",params![u.as_str(),item.label,domain,item.thumbnail,extracted.published,format!("{} {}",extracted.description,extracted.metadata)])?;
                } else {
                    tx.execute("INSERT OR IGNORE INTO staging_images(url,referring_url,title,alt,domain,published_at) VALUES(?1,?2,?3,?4,?5,?6)",params![u.as_str(),url.as_str(),extracted.title,item.label,domain,extracted.published])?;
                }
            }
            tx.execute(
                "INSERT OR IGNORE INTO staging_domains(registrable,popularity) VALUES(?1,0)",
                [&reg],
            )?;
            tx.commit()?;
        }
        if !extracted.nofollow {
            for link in &extracted.links {
                let Ok(destination) = Url::parse(link) else {
                    continue;
                };
                if self.eligible(&destination).is_none() {
                    let regdst = registrable(destination.host_str().unwrap());
                    self.state.db().execute("INSERT INTO staging_domains(registrable,popularity) VALUES(?1,1) ON CONFLICT(registrable) DO UPDATE SET popularity=popularity+1",[&regdst])?;
                    self.enqueue_discovered(link, task.depth, &reg)?;
                } else {
                    self.enqueue(link, 0)?;
                }
            }
        }
        tracing::debug!(elapsed_ms = started.elapsed().as_millis(), "page indexed");
        Ok(())
    }
}

#[derive(Clone)]
struct Media {
    url: String,
    label: String,
    video: bool,
    thumbnail: Option<String>,
}
#[derive(Default)]
struct Extracted {
    title: String,
    description: String,
    text: String,
    language: Option<String>,
    published: Option<i64>,
    canonical: Option<String>,
    sitemaps: Vec<String>,
    news: bool,
    noindex: bool,
    nofollow: bool,
    metadata: String,
    links: Vec<String>,
    images: Vec<Media>,
    videos: Vec<Media>,
}
fn selector(s: &str) -> Selector {
    Selector::parse(s).expect("static HTML selector")
}
fn meta(doc: &Html, name: &str) -> Option<String> {
    doc.select(&selector("meta"))
        .find(|e| {
            e.value()
                .attr("name")
                .or_else(|| e.value().attr("property"))
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .and_then(|e| e.value().attr("content"))
        .map(str::to_owned)
}
fn date(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|d| d.timestamp())
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|d| d.and_utc().timestamp())
        })
}
fn extract(doc: &Html, url: &Url, noindex: bool, nofollow: bool) -> Extracted {
    let base = doc
        .select(&selector("base[href]"))
        .next()
        .and_then(|e| e.value().attr("href"))
        .and_then(|href| url.join(href).ok())
        .filter(|u| is_valid_https_url(u.as_str()))
        .unwrap_or_else(|| url.clone());
    let mut out = Extracted {
        noindex,
        nofollow,
        ..Extracted::default()
    };
    out.title = doc
        .select(&selector("title"))
        .next()
        .map(|e| e.text().collect::<String>().trim().to_owned())
        .unwrap_or_default();
    out.description = meta(doc, "description")
        .or_else(|| meta(doc, "og:description"))
        .unwrap_or_default();
    out.language = doc
        .select(&selector("html"))
        .next()
        .and_then(|e| e.value().attr("lang"))
        .map(str::to_owned);
    out.published = meta(doc, "article:published_time")
        .as_deref()
        .and_then(date);
    out.news = meta(doc, "og:type").is_some_and(|s| s.eq_ignore_ascii_case("article"))
        || doc.select(&selector("article")).next().is_some();
    for e in doc.select(&selector("meta[name]")) {
        if e.value()
            .attr("name")
            .is_some_and(|n| n.eq_ignore_ascii_case("robots") || n.eq_ignore_ascii_case(AGENT))
        {
            let (i, f) = robot_directives(e.value().attr("content").unwrap_or(""));
            out.noindex |= i;
            out.nofollow |= f;
        }
    }
    let mut canonical_hints = Vec::new();
    for link in doc.select(&selector("link[href][rel]")) {
        let relations = link.value().attr("rel").unwrap_or("");
        if relations
            .split_whitespace()
            .any(|rel| rel.eq_ignore_ascii_case("canonical"))
        {
            if let Some(hint) = link
                .value()
                .attr("href")
                .and_then(|href| resolve(&base, href))
            {
                canonical_hints.push(hint);
            }
        }
        if relations
            .split_whitespace()
            .any(|rel| rel.eq_ignore_ascii_case("sitemap"))
        {
            if let Some(map) = link
                .value()
                .attr("href")
                .and_then(|href| resolve(&base, href))
            {
                out.sitemaps.push(map);
            }
        }
    }
    canonical_hints.sort();
    canonical_hints.dedup();
    if canonical_hints.len() == 1 {
        out.canonical = canonical_hints.pop();
    }
    // Read actual text nodes, excluding executable/style and navigation boilerplate.
    for body in doc.select(&selector("body")) {
        for node in body.descendants() {
            if let Some(text) = node.value().as_text() {
                let excluded = node
                    .ancestors()
                    .filter_map(scraper::ElementRef::wrap)
                    .any(|e| {
                        matches!(
                            e.value().name(),
                            "script"
                                | "style"
                                | "noscript"
                                | "nav"
                                | "header"
                                | "footer"
                                | "template"
                        )
                    });
                if !excluded {
                    out.text.push_str(text);
                    out.text.push(' ');
                }
            }
        }
    }
    out.text = out.text.split_whitespace().collect::<Vec<_>>().join(" ");
    for e in doc.select(&selector("a[href]")) {
        if e.value()
            .attr("rel")
            .unwrap_or("")
            .split_whitespace()
            .any(|s| s.eq_ignore_ascii_case("nofollow"))
        {
            continue;
        }
        if let Some(u) = e.value().attr("href").and_then(|s| resolve(&base, s)) {
            out.links.push(u);
        }
    }
    for e in doc.select(&selector("img[src]")) {
        if let Some(u) = e.value().attr("src").and_then(|s| resolve(&base, s)) {
            let label = e.value().attr("alt").unwrap_or("").to_owned();
            out.metadata.push_str(&label);
            out.metadata.push(' ');
            out.images.push(Media {
                url: u,
                label,
                video: false,
                thumbnail: None,
            });
        }
    }
    let thumbnail = meta(doc, "og:image")
        .and_then(|s| resolve(&base, &s))
        .filter(|s| is_valid_https_url(s));
    for e in doc.select(&selector("iframe[src]")) {
        if let Some(u) = e
            .value()
            .attr("src")
            .and_then(|s| resolve(&base, s))
            .and_then(|s| Url::parse(&s).ok())
        {
            let host = u.host_str().unwrap_or("");
            if ((host == "www.youtube.com"
                || host == "youtube.com"
                || host == "www.youtube-nocookie.com")
                && u.path().starts_with("/embed/"))
                || host == "player.vimeo.com"
            {
                out.videos.push(Media {
                    url: u.to_string(),
                    label: out.title.clone(),
                    video: true,
                    thumbnail: thumbnail.clone(),
                });
            }
        }
    }
    if meta(doc, "og:type").is_some_and(|s| s.starts_with("video")) {
        out.videos.push(Media {
            url: url.to_string(),
            label: out.title.clone(),
            video: true,
            thumbnail: thumbnail.clone(),
        });
    }
    for script in doc.select(&selector("script[type='application/ld+json']")) {
        if let Ok(value) =
            serde_json::from_str::<serde_json::Value>(&script.text().collect::<String>())
        {
            structured(&value, url, &mut out, thumbnail.as_deref());
        }
    }
    if !out.metadata.is_empty() {
        out.text.push(' ');
        out.text.push_str(&out.metadata);
    }
    out
}
fn structured(
    value: &serde_json::Value,
    url: &Url,
    out: &mut Extracted,
    fallback_thumbnail: Option<&str>,
) {
    if let Some(array) = value.as_array() {
        for v in array {
            structured(v, url, out, fallback_thumbnail);
        }
        return;
    }
    if let Some(graph) = value.get("@graph") {
        structured(graph, url, out, fallback_thumbnail);
    }
    let types = value
        .get("@type")
        .map(|v| {
            v.as_array().map_or_else(
                || vec![v.as_str().unwrap_or("")],
                |a| a.iter().filter_map(|v| v.as_str()).collect(),
            )
        })
        .unwrap_or_default();
    let article = types.iter().any(|t| {
        matches!(
            *t,
            "Article" | "NewsArticle" | "BlogPosting" | "ReportageNewsArticle"
        )
    });
    let video = types.contains(&"VideoObject");
    if article || video {
        if out.published.is_none() {
            out.published = value
                .get("datePublished")
                .or_else(|| value.get("uploadDate"))
                .and_then(|v| v.as_str())
                .and_then(date);
        }
        for key in ["name", "headline", "description"] {
            if let Some(s) = value.get(key).and_then(|v| v.as_str()) {
                out.metadata.push_str(s);
                out.metadata.push(' ');
            }
        }
    }
    out.news |= article;
    if article {
        if out.title.is_empty() {
            out.title = value
                .get("headline")
                .or_else(|| value.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
        }
        if out.description.is_empty() {
            out.description = value
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
        }
    }
    if video {
        let video_url = value
            .get("embedUrl")
            .or_else(|| value.get("url"))
            .and_then(|v| v.as_str())
            .and_then(|s| resolve(url, s))
            .unwrap_or_else(|| url.to_string());
        let thumbnail = value
            .get("thumbnailUrl")
            .and_then(|v| {
                v.as_str().or_else(|| {
                    v.as_array()
                        .and_then(|a| a.first())
                        .and_then(|v| v.as_str())
                })
            })
            .or(fallback_thumbnail)
            .and_then(|s| resolve(url, s))
            .filter(|s| is_valid_https_url(s));
        out.videos.push(Media {
            url: video_url,
            label: value
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&out.title)
                .to_owned(),
            video: true,
            thumbnail,
        });
    }
}

fn robot_directives(raw: &str) -> (bool, bool) {
    let mut noindex = false;
    let mut nofollow = false;
    let mut applicable = true;
    for part in raw.split(',') {
        let mut value = part.trim();
        if let Some((agent, directives)) = value.split_once(':') {
            if matches!(
                agent.trim().to_ascii_lowercase().as_str(),
                "unavailable_after" | "max-snippet" | "max-image-preview" | "max-video-preview"
            ) {
                continue;
            }
            applicable = agent.trim().eq_ignore_ascii_case(AGENT) || agent.trim() == "*";
            value = directives;
        }
        if applicable {
            for token in value.split_whitespace() {
                match token.to_ascii_lowercase().as_str() {
                    "noindex" => noindex = true,
                    "nofollow" => nofollow = true,
                    "none" => {
                        noindex = true;
                        nofollow = true;
                    }
                    _ => {}
                }
            }
        }
    }
    (noindex, nofollow)
}
fn robots_extensions(rules: &str) -> (f64, Vec<String>) {
    let mut groups: Vec<(Vec<String>, f64)> = Vec::new();
    let mut agents = Vec::new();
    let mut delay: f64 = 0.0;
    let mut has_rules = false;
    let mut sitemaps = Vec::new();
    for line in rules.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "sitemap" => sitemaps.push(value.to_owned()),
            "user-agent" => {
                if has_rules {
                    groups.push((std::mem::take(&mut agents), delay));
                    delay = 0.0;
                    has_rules = false;
                }
                agents.push(value.to_ascii_lowercase());
            }
            "crawl-delay" => {
                has_rules = true;
                if let Ok(n) = value.parse::<f64>() {
                    if n.is_finite() {
                        delay = delay.max(n.max(0.0).min(u32::MAX as f64));
                    }
                }
            }
            "request-rate" => {
                has_rules = true;
                if let Some((requests, seconds)) = value.split_once('/') {
                    if let (Ok(n), Ok(s)) = (
                        requests.trim().parse::<f64>(),
                        seconds.trim().parse::<f64>(),
                    ) {
                        if n > 0.0 && s.is_finite() {
                            delay = delay.max((s / n).clamp(0.0, u32::MAX as f64));
                        }
                    }
                }
            }
            "allow" | "disallow" => has_rules = true,
            _ => {}
        }
    }
    groups.push((agents, delay));
    let specific = groups.iter().any(|(a, _)| {
        a.iter()
            .any(|s| !s.is_empty() && s != "*" && AGENT.to_ascii_lowercase().contains(s))
    });
    let delay = groups
        .iter()
        .filter(|(a, _)| {
            a.iter().any(|s| {
                if specific {
                    !s.is_empty() && s != "*" && AGENT.to_ascii_lowercase().contains(s)
                } else {
                    s == "*"
                }
            })
        })
        .map(|(_, d)| *d)
        .fold(0.0, f64::max);
    (delay, sitemaps)
}
fn robots_url_allowed(rules: &str, url: &Url) -> bool {
    robotstxt::DefaultMatcher::default().one_agent_allowed_by_robots(rules, AGENT, url.as_str())
}
pub fn robots_text_allowed_pub(rules: &str, path: &str) -> bool {
    robots_url_allowed(
        rules,
        &Url::parse(&format!("https://example.com{path}")).unwrap(),
    )
}
fn registrable(host: &str) -> String {
    psl::domain_str(host).unwrap_or(host).to_owned()
}
fn child_depth(scope: &str, parent_depth: u32, target: &Url) -> u32 {
    if registrable(target.host_str().unwrap_or("")) == scope {
        parent_depth.saturating_add(1)
    } else {
        0
    }
}
fn resolve(url: &Url, href: &str) -> Option<String> {
    normalize_url(url.join(href).ok()?.as_str())
}
pub fn normalize_url(value: &str) -> Option<String> {
    let mut url = Url::parse(value).ok()?;
    url.set_fragment(None);
    Some(url.to_string())
}
fn is_valid_https_url(value: &str) -> bool {
    Url::parse(value).ok().is_some_and(|u| {
        u.scheme() == "https"
            && matches!(u.host(), Some(url::Host::Domain(_)))
            && u.username().is_empty()
            && u.password().is_none()
    })
}
pub fn is_valid_https_url_pub(value: &str) -> bool {
    is_valid_https_url(value)
}
pub fn banned_word_matches(word: &str, mode: &str, haystack: &str) -> bool {
    if word.trim().is_empty() {
        return false;
    }
    let lower = haystack.to_lowercase();
    let needle = word.to_lowercase();
    if mode == "substring" {
        return lower.contains(&needle);
    }
    lower.match_indices(&needle).any(|(start, _)| {
        let end = start + needle.len();
        !lower[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
            && !lower[end..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}
enum BodyError {
    Oversized,
    Network,
}
async fn read_body(
    mut response: reqwest::Response,
    limit: usize,
) -> std::result::Result<Vec<u8>, BodyError> {
    if limit != 0 && response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(BodyError::Oversized);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| BodyError::Network)? {
        if limit != 0 && chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(BodyError::Oversized);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub fn finalize(state: &AppState) -> Result<()> {
    publish(state, None).map(|_| ())
}

fn publish(state: &AppState, cancel: Option<&CancellationToken>) -> Result<bool> {
    let mut conn = state.db();
    publish_on(state, &mut conn, cancel)
}

fn publish_on(
    state: &AppState,
    conn: &mut rusqlite::Connection,
    cancel: Option<&CancellationToken>,
) -> Result<bool> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        return Ok(false);
    }
    let tx = conn.transaction()?;
    // FTS indexes and every associated published table change in one transaction.
    // Any SQL/FTS error rolls back the entire replacement, including history.
    tx.execute_batch("DELETE FROM pages; DELETE FROM images; DELETE FROM videos; DELETE FROM news; DELETE FROM domains; DELETE FROM refused_urls; DELETE FROM http_responses; DELETE FROM robots;
        INSERT INTO pages(url,domain,title,description,text,language,published_at,crawled_at) SELECT url,domain,title,description,text,language,published_at,crawled_at FROM staging_pages;
        INSERT INTO images(url,referring_url,title,alt,domain,published_at) SELECT url,referring_url,title,alt,domain,published_at FROM staging_images;
        INSERT INTO videos(url,title,domain,thumbnail_url,published_at,description) SELECT url,title,domain,thumbnail_url,published_at,description FROM staging_videos;
        INSERT INTO news(url,title,domain,published_at,description) SELECT url,title,domain,published_at,description FROM staging_news;
        INSERT INTO domains SELECT * FROM staging_domains;
        INSERT INTO refused_urls(url,domain,reason,status,ts) SELECT url,domain,reason,status,ts FROM staging_refused_urls;
        INSERT INTO http_responses(url,status,error,duration_ms) SELECT url,status,error,duration_ms FROM staging_http_responses;
        INSERT INTO robots SELECT * FROM staging_robots;")?;
    crate::canonical::consolidate(&tx)?;
    tx.execute_batch("INSERT INTO pages_fts(pages_fts) VALUES('integrity-check');
        INSERT INTO images_fts(images_fts) VALUES('rebuild'); INSERT INTO videos_fts(videos_fts) VALUES('rebuild'); INSERT INTO news_fts(news_fts) VALUES('rebuild');")?;
    let now = chrono::Utc::now().timestamp();
    let (processed, start) = {
        let g = state.inner.lock().unwrap();
        (g.processed, g.crawl_start.unwrap_or(now))
    };
    let duration = (now - start).max(0) * 1000;
    let average: f64 = tx.query_row(
        "SELECT COALESCE(avg(duration_ms),0.0) FROM http_responses",
        [],
        |r| r.get(0),
    )?;
    let mut statuses = HashMap::new();
    for row in tx
        .prepare("SELECT status,count(*) FROM http_responses GROUP BY status")?
        .query_map([], |r| {
            Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, i64>(1)?))
        })?
    {
        let (status, count) = row?;
        statuses.insert(
            status.map_or_else(|| "network_error".into(), |s| s.to_string()),
            count,
        );
    }
    let details = serde_json::json!({"average_retrieval_ms":average,"http_statuses":statuses});
    tx.execute("INSERT INTO crawl_history(completed_at,duration_ms,domains,pages,images,videos,news,refused,processed,details)
        SELECT ?1,?2,(SELECT count(DISTINCT domain) FROM pages),(SELECT count(*) FROM pages),(SELECT count(*) FROM images),(SELECT count(*) FROM videos),(SELECT count(*) FROM news),(SELECT count(*) FROM refused_urls),?3,?4",params![now,duration,processed,details.to_string()])?;
    // Cancellation during a large copy/FTS rebuild must still roll back, rather
    // than publishing simply because finalization had already started.
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        return Ok(false);
    }
    tx.commit()?;
    let mut g = state.inner.lock().unwrap();
    g.last_success = Some(now);
    g.last_duration_ms = Some(duration);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extraction_respects_content_and_nofollow() {
        let doc=Html::parse_document("<html lang='fr'><head><title>News</title><script>bad script</script></head><body><nav>navigation</nav><article>café</article><a rel='nofollow' href='/private'>hidden</a><a href='/ok#part'>link</a></body></html>");
        let out = extract(
            &doc,
            &Url::parse("https://example.com/").unwrap(),
            false,
            false,
        );
        assert!(out.text.contains("café"));
        assert!(!out.text.contains("bad script"));
        assert!(!out.text.contains("navigation"));
        assert!(out.news);
        assert_eq!(out.links, vec!["https://example.com/ok"]);
    }
    #[test]
    fn structured_metadata_and_base_urls_are_indexable() {
        let doc = Html::parse_document(
            r#"<head><base href='/articles/'><title>Site</title><script type='application/ld+json'>{"@type":"NewsArticle","headline":"Community headline","datePublished":"2026-10-30"}</script></head><body><a href='next'>Next</a></body>"#,
        );
        let out = extract(
            &doc,
            &Url::parse("https://example.com/").unwrap(),
            false,
            false,
        );
        assert!(out.text.contains("Community headline"));
        assert!(out.news);
        assert!(out.published.is_some());
        assert_eq!(out.links, vec!["https://example.com/articles/next"]);
    }
    #[test]
    fn robot_groups_and_delays() {
        assert_eq!(robots_extensions("User-agent: *\nCrawl-delay: 3\nUser-agent: SSSE\nCrawl-delay: 5\nRequest-rate: 1/8").0,8.0);
        assert!(robots_text_allowed_pub(
            "User-agent: *\nAllow: /\nDisallow: /private",
            "/"
        ));
        assert!(!robots_text_allowed_pub(
            "User-agent: *\nAllow: /\nDisallow: /private",
            "/private"
        ));
    }
    #[test]
    fn robot_header_tokens() {
        assert_eq!(
            robot_directives("otherbot: noindex, SSSE: none"),
            (true, true)
        );
        assert_eq!(robot_directives("otherbot: noindex"), (false, false));
    }

    struct Fixture {
        origin: String,
        client: reqwest::Client,
        hits: Arc<Mutex<Vec<(String, Instant)>>>,
        listener: tokio::task::JoinHandle<()>,
        robots: Arc<Mutex<String>>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.listener.abort();
        }
    }
    async fn fixture(robots: &str) -> Fixture {
        let rcgen::CertifiedKey { cert, key_pair } = rcgen::generate_simple_self_signed(vec![
            "fixture.test".into(),
            "maps.test".into(),
            "outside.test".into(),
        ])
        .unwrap();
        let config = tokio_rustls::rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                tokio_rustls::rustls::pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der())
                    .into(),
            )
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let origin = format!("https://fixture.test:{}", addr.port());
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .add_root_certificate(reqwest::Certificate::from_der(cert.der()).unwrap())
            .resolve("fixture.test", addr)
            .resolve("maps.test", addr)
            .resolve("outside.test", addr)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let server_hits = hits.clone();
        let robots = Arc::new(Mutex::new(robots.replace("ORIGIN", &origin)));
        let server_robots = robots.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let hits = server_hits.clone();
                let robots = server_robots.clone();
                tokio::spawn(async move {
                    let Ok(stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    let service = hyper::service::service_fn(
                        move |request: hyper::Request<hyper::body::Incoming>| {
                            let hits = hits.clone();
                            let robots = robots.lock().unwrap().clone();
                            async move {
                                let path = request.uri().path();
                                hits.lock()
                                    .unwrap()
                                    .push((path.to_string(), Instant::now()));
                                let mut response = hyper::Response::builder().status(200);
                                let body=match path {
                                "/robots.txt"=>{
                                    response=response.header("content-type","text/plain");
                                    if let Some(status)=robots.strip_prefix("status:") {response=response.status(status.parse::<u16>().unwrap());String::new()}
                                    else if robots.starts_with("redirect:") {response=response.status(302).header("location","/robots-1");String::new()}
                                    else {robots}
                                },
                                "/"=>"<html><head><title>Community café</title><meta property='og:type' content='article'><meta property='article:published_time' content='2026-10-30T10:00:00Z'></head><body><article>Community news</article><a href='/allowed'>one</a><a href='/allowed#fragment'>two</a><a rel='nofollow' href='/hidden'>hidden</a><a href='/blocked'>blocked</a><a href='/redirect'>redirect</a><a href='/attachment'>file</a><a href='/oversized'>large</a><a href='/unicode'>text</a><a href='/banned'>banned</a><img src='/image.png' alt='community photo'></body></html>".into(),
                                "/allowed"=>"<title>Allowed</title><body><a href='/deep'>deeper</a>hello world</body>".into(),
                                "/banned"=>"<body>forbiddenword<a href='/should-not-follow'>link</a></body>".into(),
                                "/redirect"=>{response=response.status(302).header("location","/redirect-target");String::new()},
                                "/attachment"=>{response=response.header("content-disposition","attachment; filename=\"test.html\"");"<body>attachment</body>".into()},
                                "/oversized"=>{response=response.header("transfer-encoding","chunked");"x".repeat(4096)},
                                "/unicode"=>format!("<body>{}</body>","é".repeat(101)),
                                "/meta-noindex"=>"<head><meta name='ROBOTS' content='none'></head><body><a href='/never'>do not follow</a></body>".into(),
                                "/header-noindex"=>{response=response.header("x-robots-tag","otherbot: nofollow").header("x-robots-tag","noindex");"<body>do not index</body>".into()},
                                "/header-nofollow"=>{response=response.header("x-robots-tag","otherbot: noindex").header("x-robots-tag","nofollow");"<body><a href='/never'>do not follow</a></body>".into()},
                                "/sitemap.xml"=>{response=response.header("content-type","application/xml");"<urlset xmlns='http://www.sitemaps.org/schemas/sitemap/0.9'><url><loc>ORIGIN/mapped?a=1&amp;b=2</loc></url></urlset>".replace("ORIGIN",&format!("https://fixture.test:{}",addr.port()))},
                                "/mapped"=>"<title>Mapped</title><body>from sitemap</body>".into(),
                                "/maps/empty.xml"=>"<urlset/>".into(),
                                "/scope"=>"<title>Scope</title><body>Scope seed</body>".into(),
                                "/maps/index.xml.gz"=>format!("<sitemapindex xmlns='http://www.sitemaps.org/schemas/sitemap/0.9'><sitemap><loc>https://maps.test:{}/maps/nested.xml</loc></sitemap></sitemapindex>",addr.port()),
                                "/maps/nested.xml"=>format!("<sitemapindex><sitemap><loc>https://fixture.test:{}/maps/leaf.xml.gz</loc></sitemap><sitemap><loc>https://fixture.test:{}/maps/index.xml.gz</loc></sitemap></sitemapindex>",addr.port(),addr.port()),
                                "/maps/leaf.xml.gz"=>format!("<urlset><url><loc>https://fixture.test:{}/level-one</loc></url><url><loc>https://outside.test:{}/outside</loc></url><url><loc>https://fixture.test:{}/many/slashes/deep/document</loc></url></urlset>",addr.port(),addr.port(),addr.port()),
                                "/level-one"=>"<body>First level<a href='/level-two'>next</a></body>".into(),
                                "/level-two"=>"<body>Second level</body>".into(),
                                "/outside"=>"<body>External entry<a href='/outside-child'>child</a></body>".into(),
                                "/outside-child"=>"<body>External child</body>".into(),
                                "/many/slashes/deep/document"=>"<body>One discovery hop</body>".into(),
                                "/map-entry"=>"<body>Entry<a href='/at-limit'>next</a></body>".into(),
                                "/at-limit"=>format!("<head><link rel='sitemap' href='/maps/deep-index.xml'></head><body>At limit<a href='/maps/href.xml'>Map</a><a href='https://maps.test:{}/maps/unknown-map'>External metadata</a></body>",addr.port()),
                                "/maps/deep-index.xml"=>format!("<sitemapindex><sitemap><loc>https://maps.test:{}/maps/deep-leaf.xml</loc></sitemap></sitemapindex>",addr.port()),
                                "/maps/deep-leaf.xml"=>format!("<urlset><url><loc>https://fixture.test:{}/too-deep</loc></url></urlset>",addr.port()),
                                "/too-deep"=>"<body>Must respect depth</body>".into(),
                                "/maps/href.xml"=>format!("<urlset><url><loc>https://fixture.test:{}/href-too-deep</loc></url></urlset>",addr.port()),
                                "/maps/unknown-map"=>{response=response.header("content-type","application/xml");format!("<urlset><url><loc>https://fixture.test:{}/crossmap-too-deep</loc></url></urlset>",addr.port())},
                                "/href-too-deep"|"/crossmap-too-deep"=>"<body>Metadata does not reset depth</body>".into(),
                                "/maps/bomb.xml.gz"=>format!("<urlset>{}</urlset>"," ".repeat(100_000)),
                                "/maps/truncated.xml.gz"|"/maps/encoded.xml"=>format!("<urlset><url><loc>https://fixture.test:{}/mapped</loc></url></urlset>",addr.port()),
                                "/maps/noindex.xml"=>{response=response.header("x-robots-tag","noindex");format!("<urlset><url><loc>https://fixture.test:{}/mapped</loc></url></urlset>",addr.port())},
                                "/maps/nofollow.xml"=>{response=response.header("x-robots-tag","nofollow");format!("<urlset><url><loc>https://fixture.test:{}/never</loc></url></urlset>",addr.port())},
                                "/canon/a"|"/canon/b"|"/canon/c"=> {
                                    let hint=if path=="/canon/a" {"<link rel='CANONICAL alternate' href='./b'>"} else if path=="/canon/b" {"<link rel='canonical' href='./c'>"} else {""};
                                    let (tag,video)=if path=="/canon/a" {("article","<meta property='og:type' content='video.other'>")} else {("div","")};
                                    format!("<head><title>Canonical</title>{hint}{video}</head><body><{tag}>Same canonical content</{tag}><img src='/image.png' alt='canon photo'></body>")
                                },
                                "/canon/unique"=>"<link rel='canonical' href='/canon/c'><body>Unique content</body>".into(),
                                "/canon/cycle-y"=>"<link rel='canonical' href='/canon/cycle-z'><body>Same cycle</body>".into(),
                                "/canon/cycle-z"=>"<link rel='canonical' href='/canon/cycle-y'><body>Same cycle</body>".into(),
                                "/canon/missing"=>"<link rel='canonical' href='/missing-target'><body>Missing target</body>".into(),
                                "/canon/blocked"=>"<link rel='canonical' href='/blocked-target'><body>Blocked target</body>".into(),
                                "/canon/noindex"=>"<link rel='canonical' href='/meta-noindex'><body>Noindex target</body>".into(),
                                "/canon/http"=>"<link rel='canonical' href='http://fixture.test/unsafe'><body>HTTP target</body>".into(),
                                "/canon/cross"=>format!("<link rel='canonical' href='https://outside.test:{}/cross-target'><body>Cross-origin hint</body>",addr.port()),
                                "/canon/ip"=>"<link rel='canonical' href='https://127.0.0.1/unsafe'><body>IP hint</body>".into(),
                                "/canon/depth-entry"=>"<body>Canonical entry<a href='/canon/depth-source'>next</a></body>".into(),
                                "/canon/depth-source"=>"<link rel='canonical' href='/canon/depth-target'><body>Depth target content</body>".into(),
                                "/canon/depth-target"=>"<body>Depth target content</body>".into(),
                                "/slow"=>{tokio::time::sleep(Duration::from_secs(30)).await;"<body>slow</body>".into()},
                                _ if path.starts_with("/robots-")=>{
                                    let hop=path.trim_start_matches("/robots-").parse::<usize>().unwrap();
                                    let max=robots.trim_start_matches("redirect:").parse::<usize>().unwrap();
                                    if hop<max {response=response.status(302).header("location",format!("/robots-{}",hop+1));String::new()}
                                    else {"User-agent: *\nDisallow: /allowed".into()}
                                },
                                _=>{response=response.status(404);String::new()},
                            };
                                if response
                                    .headers_ref()
                                    .unwrap()
                                    .get("content-type")
                                    .is_none()
                                {
                                    response = response.header("content-type", "text/html");
                                }
                                let body = if path.ends_with(".gz") || path == "/maps/encoded.xml" {
                                    use std::io::Write;
                                    let mut encoder = flate2::write::GzEncoder::new(
                                        Vec::new(),
                                        flate2::Compression::default(),
                                    );
                                    encoder.write_all(body.as_bytes()).unwrap();
                                    let mut bytes = encoder.finish().unwrap();
                                    if path == "/maps/truncated.xml.gz" {
                                        bytes.truncate(bytes.len() - 4);
                                    }
                                    if path == "/maps/encoded.xml" {
                                        response = response.header("content-encoding", "gzip");
                                    }
                                    bytes
                                } else {
                                    body.into_bytes()
                                };
                                Ok::<_, std::convert::Infallible>(
                                    response
                                        .body(http_body_util::Full::new(axum::body::Bytes::from(
                                            body,
                                        )))
                                        .unwrap(),
                                )
                            }
                        },
                    );
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Fixture {
            origin,
            client,
            hits,
            listener: task,
            robots,
        }
    }
    fn test_state() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ssse.db");
        crate::db::init(&path).unwrap();
        let state = AppState::new(path, dir.path().join("ssse.sock"), "127.0.0.1:0".into());
        (dir, state)
    }
    fn test_settings() -> Settings {
        Settings {
            worker_count: 4,
            worker_delay_ms: 0,
            domain_delay_ms: 0,
            fifo_refill_ms: 0,
            max_html_bytes: 1024,
            max_text_bytes: 200,
            max_depth: 1,
            ..Settings::default()
        }
    }
    fn test_crawl(fixture: &Fixture, state: &AppState, settings: Settings) -> Arc<Crawl> {
        Arc::new(Crawl {
            state: state.clone(),
            settings,
            client: fixture.client.clone(),
            timers: AsyncMutex::new(HashMap::new()),
            origins: AsyncMutex::new(HashMap::new()),
            banned_domains: Vec::new(),
            banned_words: Vec::new(),
            work: Arc::new(Mutex::new(Work::default())),
            sitemap_origins: Mutex::new(HashMap::new()),
        })
    }
    async fn crawl_fixture(
        fixture: &Fixture,
        paths: &[&str],
        settings: Settings,
    ) -> (tempfile::TempDir, AppState) {
        let (dir, state) = test_state();
        for path in paths {
            state
                .db()
                .execute(
                    "INSERT INTO seed_urls VALUES(?1)",
                    [format!("{}{path}", fixture.origin)],
                )
                .unwrap();
        }
        assert!(tokio::time::timeout(
            Duration::from_secs(15),
            run_with_client(
                state.clone(),
                settings,
                CancellationToken::new(),
                fixture.client.clone()
            )
        )
        .await
        .unwrap()
        .unwrap());
        (dir, state)
    }
    fn page_exists(state: &AppState, path: &str) -> bool {
        state
            .db()
            .query_row(
                "SELECT count(*) FROM pages WHERE url LIKE ?1 OR url LIKE ?2",
                params![format!("%{path}"), format!("%{path}?%")],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            > 0
    }

    #[tokio::test]
    async fn nested_compressed_sitemaps_preserve_page_depth_across_hosts() {
        for limit in [1, 0] {
            let fixture = fixture("User-agent: *\nSitemap: ORIGIN/maps/index.xml.gz").await;
            let mut settings = test_settings();
            settings.max_depth = limit;
            let (_dir, state) = crawl_fixture(&fixture, &["/scope", "/map-entry"], settings).await;
            for path in [
                "/level-one",
                "/outside",
                "/outside-child",
                "/many/slashes/deep/document",
            ] {
                assert!(page_exists(&state, path), "missing {path}");
            }
            for path in [
                "/level-two",
                "/too-deep",
                "/href-too-deep",
                "/crossmap-too-deep",
            ] {
                assert_eq!(
                    page_exists(&state, path),
                    limit == 0,
                    "limit {limit}: {path}"
                );
            }
            let hits = fixture.hits.lock().unwrap();
            for path in [
                "/maps/index.xml.gz",
                "/maps/nested.xml",
                "/maps/leaf.xml.gz",
            ] {
                assert_eq!(
                    hits.iter().filter(|(p, _)| p == path).count(),
                    1,
                    "duplicate sitemap {path}"
                );
            }
            if limit == 1 {
                assert!(!hits.iter().any(|(p, _)| [
                    "/too-deep",
                    "/level-two",
                    "/href-too-deep",
                    "/crossmap-too-deep"
                ]
                .contains(&p.as_str())));
            }
        }
    }

    #[tokio::test]
    async fn gzip_sitemaps_obey_decoded_limits_and_http_content_encoding() {
        for (path, limit, reason) in [
            ("/maps/bomb.xml.gz", 1024, Some("oversized_page")),
            ("/maps/bomb.xml.gz", 0, None),
            ("/maps/truncated.xml.gz", 1024, Some("invalid_sitemap")),
            ("/maps/encoded.xml", 1024, None),
        ] {
            let fixture = fixture("User-agent: *\nSitemap: ORIGIN/maps/empty.xml").await;
            let mut settings = test_settings();
            settings.max_html_bytes = limit;
            let (_dir, state) = crawl_fixture(&fixture, &[path], settings).await;
            if let Some(reason) = reason {
                assert_eq!(
                    state
                        .db()
                        .query_row(
                            "SELECT reason FROM refused_urls WHERE url=?1",
                            [format!("{}{path}", fixture.origin)],
                            |r| r.get::<_, String>(0)
                        )
                        .unwrap(),
                    reason
                );
            }
            if path == "/maps/encoded.xml" {
                assert!(page_exists(&state, "/mapped"));
            }
        }
    }

    #[tokio::test]
    async fn sitemap_noindex_is_metadata_only_but_nofollow_still_blocks_discovery() {
        let fixture = fixture("User-agent: *\nSitemap: ORIGIN/maps/empty.xml").await;
        let (_dir, state) = crawl_fixture(
            &fixture,
            &["/maps/noindex.xml", "/maps/nofollow.xml"],
            test_settings(),
        )
        .await;
        assert!(page_exists(&state, "/mapped"));
        assert!(!fixture
            .hits
            .lock()
            .unwrap()
            .iter()
            .any(|(p, _)| p == "/never"));
        assert_eq!(
            state
                .db()
                .query_row(
                    "SELECT count(*) FROM pages WHERE url LIKE '%/maps/%'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn fallback_and_html_sitemap_discovery_are_bounded_and_typed() {
        let fixture = fixture("User-agent: *\nAllow: /").await;
        let (_dir, state) = crawl_fixture(&fixture, &["/map-entry"], test_settings()).await;
        assert!(page_exists(&state, "/mapped"));
        assert!(!page_exists(&state, "/too-deep"));
        let hits = fixture.hits.lock().unwrap();
        for path in [
            "/sitemap.xml",
            "/sitemap_index.xml",
            "/maps/deep-index.xml",
            "/maps/deep-leaf.xml",
        ] {
            assert_eq!(
                hits.iter().filter(|(p, _)| p == path).count(),
                1,
                "missing or repeated probe {path}"
            );
        }
    }

    #[tokio::test]
    async fn canonical_targets_are_verified_and_duplicates_consolidate_consistently() {
        for workers in [1, 4] {
            let fixture =
                fixture("User-agent: *\nDisallow: /blocked-target\nSitemap: ORIGIN/maps/empty.xml")
                    .await;
            let mut settings = test_settings();
            settings.max_depth = 0;
            settings.worker_count = workers;
            let (_dir, state) = crawl_fixture(
                &fixture,
                &[
                    "/canon/a",
                    "/canon/unique",
                    "/canon/cycle-z",
                    "/canon/missing",
                    "/canon/blocked",
                    "/canon/noindex",
                    "/canon/http",
                    "/canon/cross",
                    "/canon/ip",
                ],
                settings,
            )
            .await;
            assert!(page_exists(&state, "/canon/c"));
            assert!(!page_exists(&state, "/canon/a"));
            assert!(!page_exists(&state, "/canon/b"));
            assert!(page_exists(&state, "/canon/cycle-y"));
            assert!(!page_exists(&state, "/canon/cycle-z"));
            for path in [
                "/canon/unique",
                "/canon/missing",
                "/canon/blocked",
                "/canon/noindex",
                "/canon/http",
                "/canon/cross",
                "/canon/ip",
            ] {
                assert!(page_exists(&state, path), "unverified hint removed {path}");
            }
            let conn = state.db();
            for table in ["news", "videos"] {
                assert_eq!(
                    conn.query_row(&format!("SELECT url FROM {table}"), [], |r| r
                        .get::<_, String>(0))
                        .unwrap(),
                    format!("{}/canon/c", fixture.origin)
                );
            }
            assert_eq!(
                conn.query_row("SELECT referring_url FROM images", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                format!("{}/canon/c", fixture.origin)
            );
            assert_eq!(
                conn.query_row(
                    "SELECT count(*) FROM pages_fts WHERE pages_fts MATCH 'canonical'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
            assert_eq!(
                conn.query_row(
                    "SELECT popularity FROM domains WHERE registrable='fixture.test'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            let hits = fixture.hits.lock().unwrap();
            assert!(!hits
                .iter()
                .any(|(p, _)| p == "/blocked-target" || p == "/cross-target" || p == "/unsafe"));
        }
    }

    #[tokio::test]
    async fn canonical_discovery_cannot_bypass_depth() {
        let fixture = fixture("User-agent: *\nSitemap: ORIGIN/maps/empty.xml").await;
        let (_dir, state) = crawl_fixture(&fixture, &["/canon/depth-entry"], test_settings()).await;
        assert!(page_exists(&state, "/canon/depth-source"));
        assert!(!page_exists(&state, "/canon/depth-target"));
        assert!(!fixture
            .hits
            .lock()
            .unwrap()
            .iter()
            .any(|(p, _)| p == "/canon/depth-target"));
    }

    #[tokio::test]
    async fn refreshed_robots_purge_pending_sqlite_and_fifo_but_preserve_active_leases() {
        let fixture = fixture("User-agent: *\nAllow: /").await;
        let (_dir, state) = test_state();
        let crawl = test_crawl(&fixture, &state, test_settings());
        let base = Url::parse(&fixture.origin).unwrap();
        assert!(crawl.robots(&base).await.unwrap().0);
        for path in [
            "/blocked-db",
            "/blocked-fifo",
            "/blocked-active",
            "/allowed",
        ] {
            crawl
                .enqueue(&format!("{}{path}", fixture.origin), 0)
                .unwrap();
        }
        {
            let mut work = crawl.work.lock().unwrap();
            for path in ["/blocked-fifo", "/blocked-active"] {
                let url = format!("{}{path}", fixture.origin);
                let id = state
                    .db()
                    .query_row("SELECT id FROM crawl_queue WHERE url=?1", [&url], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap();
                state
                    .db()
                    .execute("UPDATE crawl_queue SET assigned=1 WHERE id=?1", [id])
                    .unwrap();
                if path == "/blocked-active" {
                    work.active.insert(id);
                } else {
                    work.fifo.push_back(Task {
                        id,
                        url,
                        depth: 0,
                        sitemap: false,
                        scope: "fixture.test".into(),
                        parent_depth: None,
                    });
                }
            }
        }
        *fixture.robots.lock().unwrap() = "User-agent: *\nDisallow: /blocked".into();
        state
            .db()
            .execute("UPDATE crawl_robots SET fetched_at=fetched_at-86401", [])
            .unwrap();
        assert!(crawl.robots(&base).await.unwrap().0);
        assert!(crawl.work.lock().unwrap().fifo.is_empty());
        assert_eq!(
            state
                .db()
                .query_row("SELECT count(*) FROM crawl_queue", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            state
                .db()
                .query_row(
                    "SELECT count(*) FROM staging_refused_urls WHERE reason='robots'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        // A repeat refresh/discovery must neither double-record nor recreate purged work.
        crawl
            .purge_robots(&fixture.origin, "User-agent: *\nDisallow: /blocked")
            .unwrap();
        crawl
            .enqueue(&format!("{}/blocked-db", fixture.origin), 0)
            .unwrap();
        assert_eq!(
            state
                .db()
                .query_row("SELECT count(*) FROM staging_refused_urls", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        let id = *crawl.work.lock().unwrap().active.iter().next().unwrap();
        crawl
            .process(&Task {
                id,
                url: format!("{}/blocked-active", fixture.origin),
                depth: 0,
                sitemap: false,
                scope: "fixture.test".into(),
                parent_depth: None,
            })
            .await
            .unwrap();
        assert_eq!(
            state
                .db()
                .query_row("SELECT count(*) FROM staging_refused_urls", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert!(!fixture
            .hits
            .lock()
            .unwrap()
            .iter()
            .any(|(p, _)| p.starts_with("/blocked")));
    }

    #[tokio::test]
    async fn waiting_worker_rechecks_refreshed_rules_before_its_request() {
        let fixture = fixture("User-agent: *\nAllow: /").await;
        let (_dir, state) = test_state();
        let crawl = test_crawl(&fixture, &state, test_settings());
        let url = Url::parse(&format!("{}/blocked-active", fixture.origin)).unwrap();
        crawl.robots(&url).await.unwrap();
        crawl.enqueue(url.as_str(), 0).unwrap();
        let id = state
            .db()
            .query_row("SELECT id FROM crawl_queue", [], |r| r.get::<_, i64>(0))
            .unwrap();
        crawl.work.lock().unwrap().active.insert(id);
        crawl.timers.lock().await.insert(
            "fixture.test".into(),
            Instant::now() + Duration::from_millis(200),
        );
        let worker = crawl.clone();
        let handle = tokio::spawn(async move {
            worker
                .process(&Task {
                    id,
                    url: url.to_string(),
                    depth: 0,
                    sitemap: false,
                    scope: "fixture.test".into(),
                    parent_depth: None,
                })
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        *fixture.robots.lock().unwrap() = "User-agent: *\nDisallow: /blocked".into();
        state
            .db()
            .execute("UPDATE crawl_robots SET fetched_at=fetched_at-86401", [])
            .unwrap();
        crawl
            .robots(&Url::parse(&fixture.origin).unwrap())
            .await
            .unwrap();
        handle.await.unwrap().unwrap();
        assert!(!fixture
            .hits
            .lock()
            .unwrap()
            .iter()
            .any(|(p, _)| p == "/blocked-active"));
        assert_eq!(
            state
                .db()
                .query_row("SELECT count(*) FROM staging_refused_urls", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn cancellation_during_publication_rolls_back() {
        let (_dir, state) = test_state();
        let mut conn = state.db();
        conn.execute("INSERT INTO pages(url,domain,title,crawled_at) VALUES('https://old.example/','old.example','old',1)",[]).unwrap();
        conn.execute("INSERT INTO staging_pages(url,domain,title,crawled_at) VALUES('https://new.example/','new.example','new',1)",[]).unwrap();
        let cancel = CancellationToken::new();
        let trigger_cancel = cancel.clone();
        conn.create_scalar_function(
            "cancel_publication",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                trigger_cancel.cancel();
                Ok(0)
            },
        )
        .unwrap();
        conn.execute_batch("CREATE TRIGGER cancel_publication AFTER INSERT ON pages BEGIN SELECT cancel_publication(); END;").unwrap();
        assert!(!publish_on(&state, &mut conn, Some(&cancel)).unwrap());
        assert!(cancel.is_cancelled());
        assert_eq!(
            conn.query_row("SELECT title FROM pages", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "old"
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM crawl_history", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(state.inner.lock().unwrap().last_success.is_none());
    }
    #[tokio::test]
    async fn https_crawl_enforces_restrictions_and_indexes_references() {
        let fixture =
            fixture("User-agent: *\nDisallow: /blocked\nSitemap: ORIGIN/sitemap.xml").await;
        let (_dir, state) = test_state();
        state
            .db()
            .execute(
                "INSERT INTO seed_urls VALUES(?1)",
                [format!("{}/", fixture.origin)],
            )
            .unwrap();
        state
            .db()
            .execute(
                "INSERT INTO banned_words VALUES('forbiddenword','whole')",
                [],
            )
            .unwrap();
        assert!(run_with_client(
            state.clone(),
            test_settings(),
            CancellationToken::new(),
            fixture.client.clone()
        )
        .await
        .unwrap());
        let conn = state.db();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM pages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM images", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM news", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        for reason in [
            "robots",
            "redirect",
            "attachment",
            "oversized_page",
            "oversized_text",
            "banned_word",
            "max_depth",
        ] {
            assert!(
                conn.query_row(
                    "SELECT count(*) FROM refused_urls WHERE reason=?1",
                    [reason],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap()
                    > 0,
                "missing refusal {reason}"
            );
        }
        let hits = fixture.hits.lock().unwrap();
        for path in [
            "/hidden",
            "/blocked",
            "/redirect-target",
            "/image.png",
            "/deep",
            "/should-not-follow",
        ] {
            assert!(
                !hits.iter().any(|(p, _)| p == path),
                "unexpected fetch {path}"
            );
        }
        assert_eq!(hits.iter().filter(|(p, _)| p == "/allowed").count(), 1);
        assert_eq!(hits.iter().filter(|(p, _)| p == "/robots.txt").count(), 1);
    }
    #[tokio::test]
    async fn cancellation_does_not_publish_and_zero_limits_are_unlimited() {
        let fixture = fixture("User-agent: *\nAllow: /").await;
        let (_dir, state) = test_state();
        state.db().execute("INSERT INTO pages(url,domain,title,crawled_at) VALUES('https://old.example/','old.example','old',1)",[]).unwrap();
        state
            .db()
            .execute(
                "INSERT INTO seed_urls VALUES(?1)",
                [format!("{}/slow", fixture.origin)],
            )
            .unwrap();
        let token = CancellationToken::new();
        let cancel = token.clone();
        let stopper = async {
            loop {
                if fixture
                    .hits
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(p, _)| p == "/slow")
                {
                    cancel.cancel();
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        let (result, _) = tokio::join!(
            run_with_client(
                state.clone(),
                test_settings(),
                token,
                fixture.client.clone()
            ),
            stopper
        );
        assert!(!result.unwrap());
        assert_eq!(
            state
                .db()
                .query_row("SELECT title FROM pages", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "old"
        );
        crate::db::cleanup(&state.db()).unwrap();
        state.db().execute("DELETE FROM seed_urls", []).unwrap();
        state
            .db()
            .execute(
                "INSERT INTO seed_urls VALUES(?1)",
                [format!("{}/unicode", fixture.origin)],
            )
            .unwrap();
        let mut settings = test_settings();
        settings.max_html_bytes = 0;
        settings.max_text_bytes = 0;
        settings.max_depth = 0;
        assert!(run_with_client(
            state.clone(),
            settings,
            CancellationToken::new(),
            fixture.client.clone()
        )
        .await
        .unwrap());
        assert_eq!(
            state
                .db()
                .query_row("SELECT length(text) FROM pages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            101
        );
    }
    #[tokio::test]
    async fn domain_cooldown_is_shared_between_workers() {
        let fixture = fixture("User-agent: *\nCrawl-delay: 0.04").await;
        let (_dir, state) = test_state();
        for path in ["/allowed", "/mapped", "/unicode"] {
            state
                .db()
                .execute(
                    "INSERT INTO seed_urls VALUES(?1)",
                    [format!("{}{path}", fixture.origin)],
                )
                .unwrap();
        }
        let mut settings = test_settings();
        settings.worker_delay_ms = 10;
        settings.domain_delay_ms = 10;
        let started = Instant::now();
        assert!(run_with_client(
            state,
            settings,
            CancellationToken::new(),
            fixture.client.clone()
        )
        .await
        .unwrap());
        // Observe total spacing, not packet arrival gaps: TLS/network scheduling
        // can compress arrivals even when request starts were correctly spaced.
        assert!(started.elapsed() >= Duration::from_millis(180));
    }

    #[tokio::test]
    async fn domain_reservations_do_not_burst_after_a_late_wakeup() {
        let (_dir, state) = test_state();
        let mut settings = test_settings();
        settings.worker_delay_ms = 10;
        settings.domain_delay_ms = 10;
        let crawl = Arc::new(Crawl {
            state,
            settings,
            client: reqwest::Client::new(),
            timers: AsyncMutex::new(HashMap::new()),
            origins: AsyncMutex::new(HashMap::new()),
            banned_domains: Vec::new(),
            banned_words: Vec::new(),
            work: Arc::new(Mutex::new(Work::default())),
            sitemap_origins: Mutex::new(HashMap::new()),
        });
        let mut tasks = tokio::task::JoinSet::new();
        for i in 0..8 {
            let crawl = crawl.clone();
            tasks.spawn(async move {
                let url = Url::parse(if i % 2 == 0 {
                    "https://a.example.com/"
                } else {
                    "https://b.example.com/"
                })
                .unwrap();
                crawl.reserve(&url, 0.04).await
            });
        }
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(150));
        let mut starts = Vec::new();
        while let Some(result) = tasks.join_next().await {
            starts.push(result.unwrap());
        }
        starts.sort();
        for pair in starts.windows(2) {
            assert!(pair[1].duration_since(pair[0]) >= Duration::from_millis(60));
        }
    }

    #[tokio::test]
    async fn robots_status_and_redirect_policy() {
        for (rules, expected) in [
            ("status:404", true),
            ("status:503", false),
            ("redirect:5", false),
            ("redirect:6", false),
        ] {
            let fixture = fixture(rules).await;
            let (_dir, state) = test_state();
            state
                .db()
                .execute(
                    "INSERT INTO seed_urls VALUES(?1)",
                    [format!("{}/allowed", fixture.origin)],
                )
                .unwrap();
            assert!(run_with_client(
                state.clone(),
                test_settings(),
                CancellationToken::new(),
                fixture.client.clone()
            )
            .await
            .unwrap());
            assert_eq!(
                fixture
                    .hits
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(p, _)| p == "/allowed"),
                expected,
                "{rules}"
            );
            assert_eq!(
                state
                    .db()
                    .query_row("SELECT origin FROM robots", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                fixture.origin
            );
            let redirects = fixture
                .hits
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _)| p.starts_with("/robots-"))
                .count();
            assert!(redirects <= 5);
            if rules == "redirect:5" {
                assert_eq!(
                    state
                        .db()
                        .query_row("SELECT status FROM robots", [], |r| r.get::<_, u16>(0))
                        .unwrap(),
                    200
                );
            }
        }
    }

    #[tokio::test]
    async fn html_and_header_robots_directives_are_enforced() {
        let fixture = fixture("User-agent: *\nAllow: /").await;
        let (_dir, state) = test_state();
        for path in ["/meta-noindex", "/header-noindex", "/header-nofollow"] {
            state
                .db()
                .execute(
                    "INSERT INTO seed_urls VALUES(?1)",
                    [format!("{}{path}", fixture.origin)],
                )
                .unwrap();
        }
        assert!(run_with_client(
            state.clone(),
            test_settings(),
            CancellationToken::new(),
            fixture.client.clone()
        )
        .await
        .unwrap());
        assert_eq!(
            state
                .db()
                .query_row("SELECT count(*) FROM pages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            state
                .db()
                .query_row(
                    "SELECT count(*) FROM refused_urls WHERE reason='noindex'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        assert!(!fixture
            .hits
            .lock()
            .unwrap()
            .iter()
            .any(|(p, _)| p == "/never"));
    }

    #[tokio::test]
    async fn robots_cache_refreshes_after_twenty_four_hours() {
        let fixture = fixture("User-agent: *\nAllow: /").await;
        let (_dir, state) = test_state();
        let crawl = Crawl {
            state: state.clone(),
            settings: test_settings(),
            client: fixture.client.clone(),
            timers: AsyncMutex::new(HashMap::new()),
            origins: AsyncMutex::new(HashMap::new()),
            banned_domains: Vec::new(),
            banned_words: Vec::new(),
            work: Arc::new(Mutex::new(Work::default())),
            sitemap_origins: Mutex::new(HashMap::new()),
        };
        let url = Url::parse(&format!("{}/allowed", fixture.origin)).unwrap();
        assert!(crawl.robots(&url).await.unwrap().0);
        state
            .db()
            .execute(
                "UPDATE crawl_robots SET rules='User-agent: *\nDisallow: /'",
                [],
            )
            .unwrap();
        assert!(!crawl.robots(&url).await.unwrap().0);
        state
            .db()
            .execute("UPDATE crawl_robots SET fetched_at=fetched_at-86401", [])
            .unwrap();
        assert!(crawl.robots(&url).await.unwrap().0);
        assert_eq!(
            fixture
                .hits
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _)| p == "/robots.txt")
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn robots_refresh_discovers_new_maps_in_the_existing_origin_context() {
        let fixture = fixture("User-agent: *\nSitemap: ORIGIN/maps/empty.xml").await;
        let (_dir, state) = test_state();
        let crawl = test_crawl(&fixture, &state, test_settings());
        let root = Url::parse(&format!("{}/", fixture.origin)).unwrap();
        crawl.robots(&root).await.unwrap();
        crawl
            .discover_sitemaps(
                &Task {
                    id: 0,
                    url: root.to_string(),
                    depth: 1,
                    sitemap: false,
                    scope: "fixture.test".into(),
                    parent_depth: Some(0),
                },
                &root,
                &[],
            )
            .unwrap();
        *fixture.robots.lock().unwrap() = format!(
            "User-agent: *\nSitemap: {}/maps/deep-index.xml",
            fixture.origin
        );
        state
            .db()
            .execute("UPDATE crawl_robots SET fetched_at=fetched_at-86401", [])
            .unwrap();
        crawl.robots(&root).await.unwrap();
        let row = state
            .db()
            .query_row(
                "SELECT depth,scope,kind FROM crawl_queue WHERE url=?1",
                [format!("{}/maps/deep-index.xml", fixture.origin)],
                |r| {
                    Ok((
                        r.get::<_, u32>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row, (1, "fixture.test".into(), "sitemap".into()));
    }
}
