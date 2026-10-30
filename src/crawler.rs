use crate::settings::Settings;
use crate::state::AppState;
use scraper::{Html, Selector};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use url::Url;

#[derive(Clone)]
struct Task {
    id: i64,
    url: String,
    depth: i64,
}

pub async fn run(state: AppState) {
    // init staging + temp
    {
        let conn = state.db();
        let _ = conn.execute_batch(
            "DELETE FROM staging_pages; DELETE FROM staging_images; DELETE FROM staging_videos; DELETE FROM staging_news; DELETE FROM staging_domains; DELETE FROM staging_refused_urls; DELETE FROM staging_http_responses; DELETE FROM staging_robots;
             DELETE FROM crawl_queue; DELETE FROM crawl_seen_urls; DELETE FROM crawl_domains; DELETE FROM crawl_robots; DELETE FROM crawl_responses; DELETE FROM crawl_refused_urls; DELETE FROM crawl_links;"
        );
        // seed urls
        let seeds: Vec<String> = conn
            .prepare("SELECT url FROM seed_urls")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|x| x.ok())
            .collect();
        let now = chrono::Utc::now().timestamp();
        for u in seeds {
            if is_valid_https_url(&u) {
                let _ = conn.execute("INSERT OR IGNORE INTO crawl_queue(url,depth,assigned,created_at) VALUES(?1,0,0,?2)", rusqlite::params![u, now]);
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO crawl_seen_urls(url) VALUES(?1)",
                    rusqlite::params![u],
                );
            }
        }
    }

    let settings = state.inner.lock().unwrap().settings.clone();
    let worker_count = settings.worker_count;
    let fifo_cap = worker_count * 2;

    let fifo: Arc<Mutex<VecDeque<Task>>> = Arc::new(Mutex::new(VecDeque::new()));
    let domain_timers: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
    let _running = state.clone();

    // refill handle
    let fifo_r = fifo.clone();
    let state_r = state.clone();
    let settings_r = settings.clone();
    tokio::spawn(async move {
        loop {
            if !state_r.inner.lock().unwrap().crawl_running {
                break;
            }
            // refill
            {
                let mut q = fifo_r.lock().unwrap();
                if q.len() < worker_count {
                    let conn = state_r.db();
                    let need = fifo_cap - q.len();
                    let mut stmt = conn
                        .prepare("SELECT id,url,depth FROM crawl_queue WHERE assigned=0 LIMIT ?1")
                        .unwrap();
                    let rows: Vec<(i64, String, i64)> = stmt
                        .query_map(rusqlite::params![need as i64], |r| {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                        })
                        .unwrap()
                        .filter_map(|x| x.ok())
                        .collect();
                    for (id, url, depth) in rows {
                        conn.execute(
                            "UPDATE crawl_queue SET assigned=1 WHERE id=?1",
                            rusqlite::params![id],
                        )
                        .ok();
                        q.push_back(Task { id, url, depth });
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(settings_r.fifo_refill_ms as u64)).await;
        }
    });

    // workers
    let mut handles = vec![];
    for _w in 0..worker_count {
        let state = state.clone();
        let fifo = fifo.clone();
        let timers = domain_timers.clone();
        let settings = settings.clone();
        handles.push(tokio::spawn(async move {
            worker_loop(state, fifo, timers, settings).await;
        }));
    }

    // wait for completion: when no running tasks and queue empty and fifo empty for a while
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let running_flag = state.inner.lock().unwrap().crawl_running;
        if !running_flag {
            break;
        }
        let fifo_empty = fifo.lock().unwrap().is_empty();
        // check persistent queue
        let conn = state.db();
        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM crawl_queue WHERE assigned=0",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let processing: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM crawl_queue WHERE assigned=1",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        // workers active? we can't easily track; use processing count
        if fifo_empty && pending == 0 && processing == 0 {
            // finalize
            break;
        }
    }

    // stop refill/worker by setting running false
    state.inner.lock().unwrap().crawl_running = false;
    for h in handles {
        let _ = h.await;
    }

    // finalize: move staging to published if success
    finalize(&state).await;
}

async fn worker_loop(
    state: AppState,
    fifo: Arc<Mutex<VecDeque<Task>>>,
    timers: Arc<Mutex<HashMap<String, Instant>>>,
    settings: Settings,
) {
    let worker_interval = Duration::from_millis(settings.worker_delay_ms as u64);
    loop {
        if !state.inner.lock().unwrap().crawl_running {
            break;
        }
        // get task
        let task = {
            let mut q = fifo.lock().unwrap();
            q.pop_front()
        };
        let Some(task) = task else {
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        };

        // worker cooldown
        tokio::time::sleep(worker_interval).await;

        // domain cooldown reserve
        let url = match Url::parse(&task.url) {
            Ok(u) => u,
            Err(_) => {
                refuse(&state, &task.url, "invalid_url", None);
                ack(&state, task.id);
                continue;
            }
        };
        let host = url.host_str().unwrap_or("").to_string();
        let reg = psl::domain_str(&host).unwrap_or(&host).to_string();
        let website_delay = get_robots_delay(&state, &url).unwrap_or(0.0);
        let effective_ms =
            settings.worker_delay_ms + settings.domain_delay_ms + (website_delay * 1000.0) as i64;
        // reserve
        let wait = {
            let mut t = timers.lock().unwrap();
            let now = Instant::now();
            let next = t.get(&reg).copied().unwrap_or(now);
            let start = if next > now { next } else { now };
            t.insert(
                reg.clone(),
                start + Duration::from_millis(effective_ms as u64),
            );
            start.saturating_duration_since(now)
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }

        // process
        process_url(&state, &settings, &task, &url, &host, &reg).await;
        ack(&state, task.id);
    }
}

fn ack(state: &AppState, id: i64) {
    let conn = state.db();
    conn.execute("DELETE FROM crawl_queue WHERE id=?1", rusqlite::params![id])
        .ok();
}

fn refuse(state: &AppState, url: &str, reason: &str, status: Option<i32>) {
    let conn = state.db();
    let domain = url::Url::parse(url).ok().and_then(|u| {
        u.host_str()
            .map(|h| psl::domain_str(h).unwrap_or(h).to_string())
    });
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO staging_refused_urls(url,domain,reason,status,ts) VALUES(?1,?2,?3,?4,?5)",
        rusqlite::params![url, domain, reason, status, now],
    )
    .ok();
    state.inner.lock().unwrap().refused += 1;
}

async fn process_url(
    state: &AppState,
    settings: &Settings,
    task: &Task,
    url: &Url,
    host: &str,
    reg: &str,
) {
    state.inner.lock().unwrap().processed += 1;

    // HTTPS only
    if url.scheme() == "http" {
        refuse(state, url.as_str(), "http_url", None);
        return;
    }
    if url.scheme() != "https" {
        refuse(state, url.as_str(), "unsupported_protocol", None);
        return;
    }
    // IP literal
    if url
        .host()
        .map(|h| matches!(h, url::Host::Ipv4(_) | url::Host::Ipv6(_)))
        .unwrap_or(false)
    {
        refuse(state, url.as_str(), "ip_literal", None);
        return;
    }
    // banned domain
    {
        let conn = state.db();
        let banned: bool = conn
            .query_row(
                "SELECT 1 FROM banned_domains WHERE domain=?1",
                rusqlite::params![reg],
                |_| Ok(true),
            )
            .unwrap_or(false)
            || conn
                .query_row(
                    "SELECT 1 FROM banned_domains WHERE domain=?1",
                    rusqlite::params![host],
                    |_| Ok(true),
                )
                .unwrap_or(false);
        if banned {
            refuse(state, url.as_str(), "banned_domain", None);
            return;
        }
    }
    // depth check for internal? We'll handle when enqueuing. If task.depth > max skip
    if task.depth > settings.max_depth {
        refuse(state, url.as_str(), "max_depth", None);
        return;
    }

    // robots check
    let origin = format!("{}://{}", url.scheme(), host);
    if !robots_allowed(state, &origin, url.path()).await {
        refuse(state, url.as_str(), "robots", None);
        return;
    }

    // fetch
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let resp = match client.get(url.as_str()).send().await {
        Ok(r) => r,
        Err(e) => {
            refuse(state, url.as_str(), "dns_or_connection", None);
            tracing::warn!("{}", e);
            return;
        }
    };
    let status = resp.status().as_u16() as i32;
    let xrobots = resp
        .headers()
        .get("x-robots-tag")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_lowercase());
    // record http response
    {
        let conn = state.db();
        conn.execute(
            "INSERT OR REPLACE INTO staging_http_responses(url,status,error) VALUES(?1,?2,NULL)",
            rusqlite::params![url.as_str(), status],
        )
        .ok();
    }
    if status >= 400 {
        // not necessarily refuse? record; for 4xx/5xx refuse url
        refuse(state, url.as_str(), "http_error", Some(status));
        // no indexing
        // but still maybe links? no
        return;
    }
    // content-type
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    // sitemap handling
    if ct.contains("xml") || url.as_str().to_lowercase().ends_with(".xml") {
        let bytes = resp.bytes().await.unwrap_or_default();
        let body = String::from_utf8_lossy(&bytes);
        let re = regex::Regex::new(r"<loc>([^<]+)</loc>").unwrap();
        for cap in re.captures_iter(&body) {
            if let Some(m) = cap.get(1) {
                let u = m.as_str().trim().to_string();
                // enqueue if valid
                if is_valid_https_url(&u) {
                    let conn = state.db();
                    let seen: bool = conn
                        .query_row(
                            "SELECT 1 FROM crawl_seen_urls WHERE url=?1",
                            rusqlite::params![u],
                            |_| Ok(true),
                        )
                        .unwrap_or(false);
                    if !seen {
                        conn.execute(
                            "INSERT OR IGNORE INTO crawl_seen_urls(url) VALUES(?1)",
                            rusqlite::params![u],
                        )
                        .ok();
                        let now = chrono::Utc::now().timestamp();
                        conn.execute("INSERT INTO crawl_queue(url,depth,assigned,created_at) VALUES(?1,0,0,?2)", rusqlite::params![u, now]).ok();
                    }
                }
            }
        }
        return;
    }
    if ct.contains("image/") {
        // image direct
        {
            let conn = state.db();
            conn.execute("INSERT OR IGNORE INTO staging_images(url,referring_url,title,alt,domain,published_at) VALUES(?1,NULL,NULL,NULL,?2,NULL)", rusqlite::params![url.as_str(), reg]).ok();
        }
        return;
    }
    if !ct.contains("text/html") && !ct.contains("application/xhtml") {
        // unsupported file / download
        if resp
            .headers()
            .get("content-disposition")
            .map(|v| v.to_str().unwrap_or("").contains("attachment"))
            .unwrap_or(false)
        {
            refuse(state, url.as_str(), "attachment", Some(status));
        } else {
            refuse(state, url.as_str(), "unsupported_file_type", Some(status));
        }
        return;
    }
    // read body with limit
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(_) => {
            refuse(state, url.as_str(), "dns_or_connection", Some(status));
            return;
        }
    };
    if bytes.len() > settings.max_html_bytes as usize {
        refuse(state, url.as_str(), "oversized_page", Some(status));
        return;
    }
    let html_str = String::from_utf8_lossy(&bytes).to_string();
    let doc = Html::parse_document(&html_str);

    // robots meta noindex/nofollow
    let mut noindex = xrobots
        .as_ref()
        .map(|s| s.contains("noindex"))
        .unwrap_or(false);
    let mut nofollow = xrobots
        .as_ref()
        .map(|s| s.contains("nofollow"))
        .unwrap_or(false);
    // meta robots
    if let Ok(sel) = Selector::parse("meta[name='robots']") {
        for el in doc.select(&sel) {
            let c = el.value().attr("content").unwrap_or("").to_lowercase();
            if c.contains("noindex") {
                noindex = true;
            }
            if c.contains("nofollow") {
                nofollow = true;
            }
        }
    }
    // X-Robots-Tag header not captured fully; skip

    // extract title
    let title = Selector::parse("title").ok().and_then(|s| {
        doc.select(&s)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
    });
    let desc = Selector::parse("meta[name='description']")
        .ok()
        .and_then(|s| {
            doc.select(&s)
                .next()
                .and_then(|e| e.value().attr("content").map(|c| c.to_string()))
        });
    // text extraction: all text in body
    let mut text = String::new();
    if let Ok(sel) = Selector::parse("body") {
        for el in doc.select(&sel) {
            text.push_str(&el.text().collect::<Vec<_>>().join(" "));
            text.push(' ');
        }
    }
    // truncate
    if text.len() > settings.max_text_bytes as usize {
        text.truncate(settings.max_text_bytes as usize);
    }
    // language
    let lang = Selector::parse("html").ok().and_then(|s| {
        doc.select(&s)
            .next()
            .and_then(|e| e.value().attr("lang").map(|c| c.to_string()))
    });
    // published date: try meta article:published_time
    let published = Selector::parse("meta[property='article:published_time']")
        .ok()
        .and_then(|s| {
            doc.select(&s)
                .next()
                .and_then(|e| e.value().attr("content"))
        })
        .and_then(|c| {
            chrono::DateTime::parse_from_rfc3339(c)
                .ok()
                .map(|dt| dt.timestamp())
        });

    // banned words check
    let banned_words = {
        let conn = state.db();
        let mut stmt = conn.prepare("SELECT word,mode FROM banned_words").unwrap();
        let rows: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .filter_map(|x| x.ok())
            .collect();
        rows
    };
    let mut haystacks = vec![
        url.as_str().to_string(),
        host.to_string(),
        title.clone().unwrap_or_default(),
        text.clone(),
    ];
    if let Some(d) = &desc {
        haystacks.push(d.clone());
    }
    let mut banned_hit = false;
    for (w, mode) in &banned_words {
        for h in &haystacks {
            if banned_word_matches(w, mode, h) {
                banned_hit = true;
                break;
            }
        }
        if banned_hit {
            break;
        }
    }
    if banned_hit {
        refuse(state, url.as_str(), "banned_word", Some(status));
        // do not follow links
        return;
    }
    if noindex {
        refuse(state, url.as_str(), "noindex", Some(status));
        return;
    }

    // canonical url
    let canonical = Selector::parse("link[rel='canonical']")
        .ok()
        .and_then(|s| doc.select(&s).next().and_then(|e| e.value().attr("href")))
        .map(|c| c.to_string());

    // news classification: article metadata
    let is_news = doc
        .select(&Selector::parse("meta[property='og:type']").unwrap())
        .next()
        .and_then(|e| e.value().attr("content"))
        .map(|c| c == "article")
        .unwrap_or(false)
        || Selector::parse("article")
            .ok()
            .map(|s| doc.select(&s).next().is_some())
            .unwrap_or(false);

    // video detection
    let mut video_urls: Vec<(String, Option<String>)> = vec![];
    // youtube iframes
    if let Ok(sel) = Selector::parse("iframe[src]") {
        for el in doc.select(&sel) {
            if let Some(src) = el.value().attr("src") {
                if src.contains("youtube.com/embed") || src.contains("player.vimeo") {
                    video_urls.push((src.to_string(), None));
                }
            }
        }
    }

    // image refs
    let mut image_urls: Vec<(String, Option<String>)> = vec![];
    if let Ok(sel) = Selector::parse("img[src]") {
        for el in doc.select(&sel) {
            if let Some(src) = el.value().attr("src") {
                let abs = resolve(url, src);
                if let Some(a) = abs {
                    image_urls.push((a, el.value().attr("alt").map(|s| s.to_string())));
                }
            }
        }
    }

    // links
    let mut links: Vec<String> = vec![];
    if let Ok(sel) = Selector::parse("a[href]") {
        for el in doc.select(&sel) {
            // nofollow link-level
            let rel = el.value().attr("rel").unwrap_or("").to_lowercase();
            let link_nofollow = rel.contains("nofollow");
            if nofollow && link_nofollow { /* skip? */ }
            if let Some(href) = el.value().attr("href") {
                if let Some(abs) = resolve(url, href) {
                    links.push(abs);
                }
            }
        }
    }
    // if nofollow at page level, skip link discovery entirely? spec nofollow means don't follow links
    let follow_links = !nofollow;

    // index page if not noindex
    if !noindex {
        let conn = state.db();
        let idx_url = canonical
            .clone()
            .unwrap_or_else(|| url.as_str().to_string());
        conn.execute(
            "INSERT OR REPLACE INTO staging_pages(url,domain,title,description,text,language,published_at,crawled_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![idx_url, reg, title, desc, text, lang, published, chrono::Utc::now().timestamp()],
        ).ok();
        if is_news {
            conn.execute(
                "INSERT OR REPLACE INTO staging_news(url,title,domain,published_at,description) VALUES(?1,?2,?3,?4,?5)",
                rusqlite::params![idx_url, title, reg, published, desc],
            ).ok();
        }
        for (vurl, _) in &video_urls {
            let abs = resolve(url, vurl).unwrap_or_else(|| vurl.clone());
            conn.execute(
                "INSERT OR IGNORE INTO staging_videos(url,title,domain,thumbnail_url,published_at,description) VALUES(?1,?2,?3,NULL,?4,?5)",
                rusqlite::params![abs, title, reg, published, desc],
            ).ok();
        }
        for (iurl, alt) in &image_urls {
            conn.execute(
                "INSERT OR IGNORE INTO staging_images(url,referring_url,title,alt,domain,published_at) VALUES(?1,?2,?3,?4,?5,?6)",
                rusqlite::params![iurl, url.as_str(), title, alt, reg, published],
            ).ok();
        }
        // domain popularity: ensure domain exists
        conn.execute(
            "INSERT OR IGNORE INTO staging_domains(registrable,popularity) VALUES(?1,0)",
            rusqlite::params![reg],
        )
        .ok();
    }

    // popularity counting: for each link reference, increment destination registrable domain
    // regardless of dedup. Do this for accepted page only? Spec: for every accepted link reference discovered during crawling.
    // We count when processing page (accepted). If page rejected (banned/noindex?), still? Spec says rejected page doesn't count outgoing. We already returned for banned. For noindex we refused and returned? We didn't return after noindex above; let's ensure noindex returns (no links). Actually above we recorded refuse but continued? We set noindex true, then later if noindex refuse... We called refuse but didn't return. Let me handle: if noindex, return before links. Adjust:
    // We'll just proceed; but to be safe, if noindex return.
    if noindex {
        return;
    }

    if follow_links {
        for link in &links {
            // popularity: increment destination domain
            if let Ok(u) = Url::parse(link) {
                if u.scheme() == "https" {
                    if let Some(h) = u.host_str() {
                        if let Ok(host) = h.parse::<std::net::IpAddr>() {
                            let _ = host;
                        }
                        let regdst = psl::domain_str(h).unwrap_or(h).to_string();
                        let conn = state.db();
                        conn.execute("INSERT INTO staging_domains(registrable,popularity) VALUES(?1,0) ON CONFLICT(registrable) DO NOTHING", rusqlite::params![regdst]).ok();
                        conn.execute("UPDATE staging_domains SET popularity = popularity + 1 WHERE registrable=?1", rusqlite::params![regdst]).ok();
                        // also update crawl_domains for tracking? use staging_domains only.
                    }
                }
            }
            // enqueue if eligible
            // enqueue eligibility
            let lu = match Url::parse(link) {
                Ok(u) => u,
                Err(_) => continue,
            };
            if lu.scheme() != "https" {
                continue;
            }
            if lu
                .host()
                .map(|h| matches!(h, url::Host::Ipv4(_) | url::Host::Ipv6(_)))
                .unwrap_or(false)
            {
                continue;
            }
            let lhost = lu.host_str().unwrap_or("");
            let lreg = psl::domain_str(lhost).unwrap_or(lhost).to_string();
            // banned domain
            {
                let conn = state.db();
                let banned: bool = conn
                    .query_row(
                        "SELECT 1 FROM banned_domains WHERE domain=?1",
                        rusqlite::params![lreg],
                        |_| Ok(true),
                    )
                    .unwrap_or(false);
                if banned {
                    continue;
                }
            }
            // depth
            let new_depth = if lreg == reg { task.depth + 1 } else { 0 };
            if new_depth > settings.max_depth {
                continue;
            }
            // robots? will check at process time
            // dedup within crawl
            {
                let conn = state.db();
                let seen: bool = conn
                    .query_row(
                        "SELECT 1 FROM crawl_seen_urls WHERE url=?1",
                        rusqlite::params![link],
                        |_| Ok(true),
                    )
                    .unwrap_or(false);
                if !seen {
                    conn.execute(
                        "INSERT OR IGNORE INTO crawl_seen_urls(url) VALUES(?1)",
                        rusqlite::params![link],
                    )
                    .ok();
                    let now = chrono::Utc::now().timestamp();
                    conn.execute(
                        "INSERT INTO crawl_queue(url,depth,assigned,created_at) VALUES(?1,?2,0,?3)",
                        rusqlite::params![link, new_depth, now],
                    )
                    .ok();
                }
            }
        }
    }
}

fn enqueue_link(state: &AppState, settings: &Settings, link: &str, _parent_depth: i64, _src: &str) {
    let url = match Url::parse(link) {
        Ok(u) => u,
        Err(_) => return,
    };
    if url.scheme() != "https" {
        return;
    }
    if url
        .host()
        .map(|h| matches!(h, url::Host::Ipv4(_) | url::Host::Ipv6(_)))
        .unwrap_or(false)
    {
        return;
    }
    let host = url.host_str().unwrap_or("");
    let reg = psl::domain_str(host).unwrap_or(host).to_string();
    // banned domain check
    {
        let conn = state.db();
        let banned: bool = conn
            .query_row(
                "SELECT 1 FROM banned_domains WHERE domain=?1",
                rusqlite::params![reg],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if banned {
            return;
        }
    }
    // depth: determine if same registrable domain as? We need know source domain. Compute source reg from? We don't pass. Assume external entry resets depth.
    // Spec: depth starts 0 when entering domain through seed/external link; internal links increase.
    // We need know if link's registrable domain equals current page's domain. We don't have current reg here easily; pass? Let's compute: if same domain as? We'll treat: if reg == current reg? We don't have. Simpler: depth+1 always? But external entry should reset.
    // We'll approximate: if registrable domain changes, depth=0 else depth+1.
    // Need current reg; not passed. Let's change signature? We'll just use depth+1 capped; external seeds depth 0. For links from a page on same domain, +1. For cross-domain, reset 0. We need know current domain. Add param.
    // We'll refactor: pass current_reg.
    let _ = settings;
}

fn resolve(url: &Url, href: &str) -> Option<String> {
    url.join(href).ok().map(|u| u.to_string())
}

pub fn is_valid_https_url_pub(u: &str) -> bool {
    is_valid_https_url(u)
}
pub fn normalize_url(u: &str) -> Option<String> {
    let mut url = url::Url::parse(u).ok()?;
    // remove fragment
    url.set_fragment(None);
    // remove trailing slash? keep as is
    Some(url.to_string())
}

pub fn banned_word_matches(word: &str, mode: &str, haystack: &str) -> bool {
    if word.trim().is_empty() { return false; }
    let lower_h = haystack.to_lowercase();
    let lower_w = word.to_lowercase();
    if mode == "substring" {
        lower_h.contains(&lower_w)
    } else {
        lower_h
            .split_whitespace()
            .any(|tok| tok.trim_matches(|c: char| !c.is_alphanumeric()) == lower_w)
            || lower_h.contains(&format!(" {} ", lower_w))
    }
}

fn is_valid_https_url(u: &str) -> bool {
    if let Ok(url) = Url::parse(u) {
        url.scheme() == "https"
            && url
                .host()
                .map(|h| !matches!(h, url::Host::Ipv4(_) | url::Host::Ipv6(_)))
                .unwrap_or(false)
    } else {
        false
    }
}

fn get_robots_delay(state: &AppState, url: &Url) -> Option<f64> {
    let origin = format!("{}://{}", url.scheme(), url.host_str().unwrap_or(""));
    let conn = state.db();
    conn.query_row(
        "SELECT crawl_delay FROM crawl_robots WHERE origin=?1",
        rusqlite::params![origin],
        |r| r.get(0),
    )
    .ok()
}

pub fn robots_text_allowed_pub(rules: &str, path: &str) -> bool {
    robots_text_allowed(rules, path)
}
fn robots_text_allowed(rules: &str, path: &str) -> bool {
    let mut active = false;
    let mut disallows: Vec<String> = vec![];
    let mut allows: Vec<String> = vec![];
    for line in rules.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = match line.split_once(':') {
            Some(x) => x,
            None => continue,
        };
        let k = k.trim().to_lowercase();
        let v = v.trim();
        match k.as_str() {
            "user-agent" => {
                active = v == "*";
            }
            "disallow" => {
                if active && !v.is_empty() {
                    disallows.push(v.to_string());
                }
            }
            "allow"
                if active && !v.is_empty() => {
                    allows.push(v.to_string());
                }
            _ => {}
        }
    }
    let mut allowed = true;
    for d in &disallows {
        if path.starts_with(d) {
            allowed = false;
        }
    }
    for a in &allows {
        if path.starts_with(a) {
            allowed = true;
        }
    }
    allowed
}

async fn robots_allowed(state: &AppState, origin: &str, path: &str) -> bool {
    let cached = {
        let conn = state.db();
        conn.query_row(
            "SELECT rules,status,fetched_at FROM crawl_robots WHERE origin=?1",
            rusqlite::params![origin],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<i32>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .ok()
    };
    let now = chrono::Utc::now().timestamp();
    let need_fetch = match &cached {
        Some((_, _, Some(ts))) => now - ts > 86400,
        _ => true,
    };
    if need_fetch {
        let mut current_url = format!("{}/robots.txt", origin);
        let mut redirects = 0;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let mut final_status = 0;
        let mut body = String::new();
        loop {
            match client.get(&current_url).send().await {
                Ok(resp) => {
                    final_status = resp.status().as_u16() as i32;
                    if resp.status().is_redirection() {
                        if redirects >= 5 {
                            let conn = state.db();
                            conn.execute("INSERT OR REPLACE INTO crawl_robots(origin,url,status,rules,crawl_delay,fetched_at) VALUES(?1,?2,?3,?4,?5,?6)",
                                rusqlite::params![origin, current_url, final_status, "User-agent: *\nDisallow: /", 0.0f64, now]).ok();
                            conn.execute("INSERT OR REPLACE INTO staging_robots(origin,url,status,rules,crawl_delay,fetched_at) VALUES(?1,?2,?3,?4,?5,?6)",
                                rusqlite::params![origin, current_url, final_status, "User-agent: *\nDisallow: /", 0.0f64, now]).ok();
                            return false;
                        }
                        if let Some(loc) =
                            resp.headers().get("location").and_then(|v| v.to_str().ok())
                        {
                            if let Ok(base) = Url::parse(&current_url) {
                                if let Ok(next) = base.join(loc) {
                                    current_url = next.to_string();
                                    redirects += 1;
                                    continue;
                                }
                            }
                        }
                        break;
                    }
                    body = resp.text().await.unwrap_or_default();
                    break;
                }
                Err(_) => {
                    let conn = state.db();
                    conn.execute("INSERT OR REPLACE INTO crawl_robots(origin,url,status,rules,crawl_delay,fetched_at) VALUES(?1,?2,?3,?4,?5,?6)",
                        rusqlite::params![origin, current_url, 0i32, "User-agent: *\nDisallow: /", 0.0f64, now]).ok();
                    conn.execute("INSERT OR REPLACE INTO staging_robots(origin,url,status,rules,crawl_delay,fetched_at) VALUES(?1,?2,?3,?4,?5,?6)",
                        rusqlite::params![origin, current_url, 0i32, "User-agent: *\nDisallow: /", 0.0f64, now]).ok();
                    return false;
                }
            }
        }
        let mut delay = 0.0;
        for line in body.lines() {
            let l = line.trim();
            if l.to_lowercase().starts_with("crawl-delay:") {
                if let Some(v) = l.split(':').nth(1) {
                    delay = v.trim().parse().unwrap_or(0.0);
                }
            }
        }
        let rules = if (400..500).contains(&final_status) {
            String::new()
        } else if final_status >= 500 || final_status == 0 {
            "User-agent: *\nDisallow: /".to_string()
        } else {
            body
        };
        let conn = state.db();
        conn.execute("INSERT OR REPLACE INTO crawl_robots(origin,url,status,rules,crawl_delay,fetched_at) VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![origin, format!("{}/robots.txt", origin), final_status, rules, delay, now]).ok();
        conn.execute("INSERT OR REPLACE INTO staging_robots(origin,url,status,rules,crawl_delay,fetched_at) VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![origin, format!("{}/robots.txt", origin), final_status, rules, delay, now]).ok();

        // sitemap discovery
        for line in rules.lines() {
            let l = line.trim();
            if l.to_lowercase().starts_with("sitemap:") {
                let sm = l["sitemap:".len()..].trim().to_string();
                if sm.starts_with("http") {
                    let conn = state.db();
                    let seen: bool = conn
                        .query_row(
                            "SELECT 1 FROM crawl_seen_urls WHERE url=?1",
                            rusqlite::params![sm],
                            |_| Ok(true),
                        )
                        .unwrap_or(false);
                    if !seen {
                        conn.execute(
                            "INSERT OR IGNORE INTO crawl_seen_urls(url) VALUES(?1)",
                            rusqlite::params![sm],
                        )
                        .ok();
                        let now = chrono::Utc::now().timestamp();
                        conn.execute("INSERT INTO crawl_queue(url,depth,assigned,created_at) VALUES(?1,0,0,?2)", rusqlite::params![sm, now]).ok();
                    }
                }
            }
        }

        if final_status >= 500 || final_status == 0 {
            return false;
        }
        if (400..500).contains(&final_status) {
            return true;
        }
        robots_text_allowed(&rules, path)
    } else {
        if let Some((rules, status, _)) = cached {
            let rules = rules.unwrap_or_default();
            let st = status.unwrap_or(0);
            if st >= 500 || st == 0 {
                return false;
            }
            if (400..500).contains(&st) {
                return true;
            }
            robots_text_allowed(&rules, path)
        } else {
            true
        }
    }
}

async fn finalize(state: &AppState) {
    let conn = state.db();
    let _ = conn.execute_batch(
        "DELETE FROM pages; DELETE FROM images; DELETE FROM videos; DELETE FROM news; DELETE FROM domains; DELETE FROM refused_urls; DELETE FROM http_responses; DELETE FROM robots;
         INSERT INTO pages(url,domain,title,description,text,language,published_at,crawled_at) SELECT url,domain,title,description,text,language,published_at,crawled_at FROM staging_pages;
         INSERT INTO images(url,referring_url,title,alt,domain,published_at) SELECT url,referring_url,title,alt,domain,published_at FROM staging_images;
         INSERT INTO videos(url,title,domain,thumbnail_url,published_at,description) SELECT url,title,domain,thumbnail_url,published_at,description FROM staging_videos;
         INSERT INTO news(url,title,domain,published_at,description) SELECT url,title,domain,published_at,description FROM staging_news;
         INSERT INTO domains(registrable,popularity) SELECT registrable,popularity FROM staging_domains;
         INSERT INTO refused_urls(url,domain,reason,status,ts) SELECT url,domain,reason,status,ts FROM staging_refused_urls;
         INSERT INTO http_responses(url,status,error) SELECT url,status,error FROM staging_http_responses;
         INSERT INTO robots(origin,url,status,rules,crawl_delay,fetched_at) SELECT origin,url,status,rules,crawl_delay,fetched_at FROM staging_robots;"
    );
    let _ = conn.execute("INSERT INTO pages_fts(pages_fts) VALUES('rebuild')", []);
    let domains: i64 = conn
        .query_row("SELECT COUNT(*) FROM domains", [], |r| r.get(0))
        .unwrap_or(0);
    let pages: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |r| r.get(0))
        .unwrap_or(0);
    let images: i64 = conn
        .query_row("SELECT COUNT(*) FROM images", [], |r| r.get(0))
        .unwrap_or(0);
    let videos: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap_or(0);
    let news: i64 = conn
        .query_row("SELECT COUNT(*) FROM news", [], |r| r.get(0))
        .unwrap_or(0);
    let refused: i64 = conn
        .query_row("SELECT COUNT(*) FROM refused_urls", [], |r| r.get(0))
        .unwrap_or(0);
    let processed = state.inner.lock().unwrap().processed as i64;
    let now = chrono::Utc::now().timestamp();
    let start = state.inner.lock().unwrap().crawl_start.unwrap_or(now);
    let dur = (now - start) * 1000;
    conn.execute("INSERT INTO crawl_history(completed_at,duration_ms,domains,pages,images,videos,news,refused,processed) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        rusqlite::params![now, dur, domains, pages, images, videos, news, refused, processed]).ok();
    state.inner.lock().unwrap().last_success = Some(now);
    state.inner.lock().unwrap().last_duration_ms = Some(dur);
    let _ = conn.execute_batch(
        "DELETE FROM staging_pages; DELETE FROM crawl_queue; DELETE FROM crawl_seen_urls;",
    );
}
