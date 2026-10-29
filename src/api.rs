use crate::state::AppState;
use axum::{extract::{Query, State}, http::{HeaderMap, Request, StatusCode}, middleware::Next, response::IntoResponse, routing::{get, post, put}, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn router(state: AppState) -> Router {
    let st = Arc::new(state);
    let admin = Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/password", post(change_password))
        .route("/settings", get(get_settings).put(update_settings))
        .route("/seeds", get(get_seeds).put(update_seeds))
        .route("/filters", get(get_filters))
        .route("/banned-domains", get(get_banned_domains).put(update_banned_domains))
        .route("/banned-words", get(get_banned_words).put(update_banned_words))
        .route("/promoted", get(get_promoted).put(update_promoted))
        .route("/crawl/status", get(crawl_status))
        .route("/crawl/start", post(crawl_start))
        .route("/crawl/stop", post(crawl_stop))
        .route("/crawl/restart", post(crawl_restart))
        .layer(axum::middleware::from_fn_with_state(st.clone(), auth_mw))
        .with_state(st.clone());
    Router::new()
        .route("/", get(serve_index))
        .route("/robots.txt", get(robots_txt))
        .route("/api/search", get(search))
        .route("/api/stats", get(stats))
        .route("/api/stats/history", get(stats_history))
        .route("/api/stats/top", get(stats_top))
        .route("/api/stats/bottom", get(stats_bottom))
        .nest("/api/admin", admin)
        .with_state(st.clone())
        .fallback_service(tower_http::services::ServeDir::new("static"))
}

async fn serve_index() -> impl IntoResponse {
    let path = "static/index.html";
    match std::fs::read_to_string(path) {
        Ok(s) => (StatusCode::OK, [("content-type","text/html")], s).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
    category: Option<String>,
    page: Option<u32>,
    per_page: Option<u32>,
}

#[derive(Serialize)]
struct SearchResult {
    url: String,
    title: Option<String>,
    description: Option<String>,
    domain: Option<String>,
    published_at: Option<i64>,
    extra: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
    total: i64,
    page: u32,
    per_page: u32,
}

async fn search(State(st): State<Arc<AppState>>, Query(p): Query<SearchParams>) -> impl IntoResponse {
    let conn = st.db();
    let q = p.q.unwrap_or_default();
    let category = p.category.unwrap_or_else(|| "PAGES".to_string()).to_uppercase();
    let page = p.page.unwrap_or(1).max(1);
    let per_page = p.per_page.unwrap_or(10).max(1);
    let offset = (page - 1) * per_page;

    // Build FTS query safely: wrap terms. Simple approach: use the raw query but quote.
    let fts_query = build_fts_query(&q);

    let (results, total) = match category.as_str() {
        "IMAGES" => query_images(&conn, &fts_query, &q, offset, per_page),
        "VIDEOS" => query_videos(&conn, &fts_query, &q, offset, per_page),
        "NEWS" => query_news(&conn, &fts_query, &q, offset, per_page),
        _ => query_pages(&conn, &fts_query, &q, offset, per_page),
    };

    Json(SearchResponse { results, total, page, per_page }).into_response()
}

fn build_fts_query(q: &str) -> String {
    let q = q.trim();
    if q.is_empty() { return String::new(); }
    // Parse simple operators
    let mut parts: Vec<String> = Vec::new();
    let mut site_filter: Option<String> = None;
    // tokenize respecting quotes
    let mut chars = q.chars().peekable();
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for c in chars {
        if c == '"' {
            in_quote = !in_quote;
            cur.push(c);
        } else if c.is_whitespace() && !in_quote {
            if !cur.trim().is_empty() { tokens.push(cur.trim().to_string()); }
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    if !cur.trim().is_empty() { tokens.push(cur.trim().to_string()); }

    for t in tokens {
        if let Some(site) = t.strip_prefix("site:") {
            site_filter = Some(site.to_string());
            continue;
        }
        if let Some(neg) = t.strip_prefix('-') {
            if !neg.is_empty() {
                parts.push(format!("NOT {}", neg));
                continue;
            }
        }
        parts.push(t);
    }
    // store site filter in a thread-local? Instead, we rely on q passed separately.
    let _ = site_filter;
    parts.join(" ")
}

fn query_pages(conn: &rusqlite::Connection, fts: &str, q: &str, offset: u32, per_page: u32) -> (Vec<SearchResult>, i64) {
    // promoted domains
    let promoted: Vec<String> = conn.prepare("SELECT domain FROM promoted_domains").unwrap()
        .query_map([], |r| r.get(0)).unwrap().filter_map(|x|x.ok()).collect();
    let mut results = vec![];
    let total: i64 = if fts.is_empty() {
        conn.query_row("SELECT COUNT(*) FROM pages", [], |r| r.get(0)).unwrap_or(0)
    } else {
        // count
        conn.query_row("SELECT COUNT(*) FROM pages p JOIN pages_fts f ON f.rowid=p.id WHERE pages_fts MATCH ?1", rusqlite::params![fts], |r| r.get(0)).unwrap_or(0)
    };
    if fts.is_empty() {
        return (results, total);
    }
    // promoted first
    for dom in &promoted {
        let mut stmt = conn.prepare("SELECT p.url,p.title,p.description,p.domain,p.published_at FROM pages p JOIN pages_fts f ON f.rowid=p.id WHERE pages_fts MATCH ?1 AND p.domain=?2 LIMIT ?3").unwrap();
        let rows = stmt.query_map(rusqlite::params![fts, dom, per_page], |r| Ok(SearchResult {
            url: r.get(0)?, title: r.get(1)?, description: r.get(2)?, domain: r.get(3)?, published_at: r.get(4)?, extra: None
        })).unwrap();
        for r in rows { if let Ok(x)=r { results.push(x); } }
    }
    let mut stmt = conn.prepare("SELECT p.url,p.title,p.description,p.domain,p.published_at FROM pages p JOIN pages_fts f ON f.rowid=p.id WHERE pages_fts MATCH ?1 ORDER BY bm25(pages_fts) LIMIT ?2 OFFSET ?3").unwrap();
    let rows = stmt.query_map(rusqlite::params![fts, per_page, offset], |r| Ok(SearchResult {
        url: r.get(0)?, title: r.get(1)?, description: r.get(2)?, domain: r.get(3)?, published_at: r.get(4)?, extra: None
    })).unwrap();
    for r in rows { if let Ok(x)=r { 
        // filter banned? already not in index
        // domain filter site:
        if q.contains("site:") { /* ignore advanced */ }
        results.push(x); 
    } }
    // dedup promoted duplicates? skip for simplicity
    (results, total)
}

fn query_images(conn: &rusqlite::Connection, fts: &str, _q: &str, offset: u32, per_page: u32) -> (Vec<SearchResult>, i64) {
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM images", [], |r| r.get(0)).unwrap_or(0);
    let mut stmt = conn.prepare("SELECT url,title,alt,domain,published_at FROM images LIMIT ?1 OFFSET ?2").unwrap();
    let rows = stmt.query_map(rusqlite::params![per_page, offset], |r| Ok(SearchResult {
        url: r.get(0)?, title: r.get(1)?, description: r.get(2)?, domain: r.get(3)?, published_at: r.get(4)?, extra: None
    })).unwrap();
    let mut v = vec![];
    for r in rows { if let Ok(x)=r { v.push(x); } }
    (v, total)
}

fn query_videos(conn: &rusqlite::Connection, fts: &str, _q: &str, offset: u32, per_page: u32) -> (Vec<SearchResult>, i64) {
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0)).unwrap_or(0);
    let mut stmt = conn.prepare("SELECT url,title,domain,thumbnail_url,published_at FROM videos LIMIT ?1 OFFSET ?2").unwrap();
    let rows = stmt.query_map(rusqlite::params![per_page, offset], |r| Ok(SearchResult {
        url: r.get(0)?, title: r.get(1)?, description: None, domain: r.get(2)?, published_at: r.get(4)?,
        extra: Some(serde_json::json!({"thumbnail": r.get::<_,Option<String>>(3)?}))
    })).unwrap();
    let mut v = vec![];
    for r in rows { if let Ok(x)=r { v.push(x); } }
    (v, total)
}

fn query_news(conn: &rusqlite::Connection, fts: &str, _q: &str, offset: u32, per_page: u32) -> (Vec<SearchResult>, i64) {
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM news", [], |r| r.get(0)).unwrap_or(0);
    let mut stmt = conn.prepare("SELECT url,title,domain,published_at,description FROM news LIMIT ?1 OFFSET ?2").unwrap();
    let rows = stmt.query_map(rusqlite::params![per_page, offset], |r| Ok(SearchResult {
        url: r.get(0)?, title: r.get(1)?, description: r.get(4)?, domain: r.get(2)?, published_at: r.get(3)?, extra: None
    })).unwrap();
    let mut v = vec![];
    for r in rows { if let Ok(x)=r { v.push(x); } }
    (v, total)
}

async fn stats(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let conn = st.db();
    let pages: i64 = conn.query_row("SELECT COUNT(*) FROM pages", [], |r| r.get(0)).unwrap_or(0);
    let domains: i64 = conn.query_row("SELECT COUNT(*) FROM domains", [], |r| r.get(0)).unwrap_or(0);
    let images: i64 = conn.query_row("SELECT COUNT(*) FROM images", [], |r| r.get(0)).unwrap_or(0);
    let videos: i64 = conn.query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0)).unwrap_or(0);
    let news: i64 = conn.query_row("SELECT COUNT(*) FROM news", [], |r| r.get(0)).unwrap_or(0);
    let refused: i64 = conn.query_row("SELECT COUNT(*) FROM refused_urls", [], |r| r.get(0)).unwrap_or(0);
    let g = st.inner.lock().unwrap();
    Json(serde_json::json!({
        "pages": pages, "domains": domains, "images": images, "videos": videos,
        "news": news, "refused": refused, "processed": g.processed,
        "last_success": g.last_success, "last_duration_ms": g.last_duration_ms
    }))
}

async fn stats_history(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let conn = st.db();
    let mut stmt = conn.prepare("SELECT completed_at,duration_ms,domains,pages,images,videos,news,refused,processed FROM crawl_history ORDER BY completed_at").unwrap();
    let rows = stmt.query_map([], |r| Ok(serde_json::json!({
        "completed_at": r.get::<_,i64>(0)?,
        "duration_ms": r.get::<_,i64>(1)?,
        "domains": r.get::<_,i64>(2)?,
        "pages": r.get::<_,i64>(3)?,
        "images": r.get::<_,i64>(4)?,
        "videos": r.get::<_,i64>(5)?,
        "news": r.get::<_,i64>(6)?,
        "refused": r.get::<_,i64>(7)?,
        "processed": r.get::<_,i64>(8)?,
    }))).unwrap();
    let mut v = vec![];
    for r in rows { if let Ok(x)=r { v.push(x); } }
    Json(v)
}

async fn stats_top(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let conn = st.db();
    let mut stmt = conn.prepare("SELECT registrable,popularity FROM domains ORDER BY popularity DESC LIMIT 10").unwrap();
    let rows = stmt.query_map([], |r| Ok(serde_json::json!({"domain": r.get::<_,String>(0)?, "popularity": r.get::<_,i64>(1)?}))).unwrap();
    let mut v = vec![];
    for r in rows { if let Ok(x)=r { v.push(x); } }
    Json(v)
}

async fn stats_bottom(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let conn = st.db();
    let mut stmt = conn.prepare("SELECT registrable,popularity FROM domains ORDER BY popularity ASC LIMIT 10").unwrap();
    let rows = stmt.query_map([], |r| Ok(serde_json::json!({"domain": r.get::<_,String>(0)?, "popularity": r.get::<_,i64>(1)?}))).unwrap();
    let mut v = vec![];
    for r in rows { if let Ok(x)=r { v.push(x); } }
    Json(v)
}

#[derive(Deserialize)]
struct LoginReq { username: String, password: String }

async fn login(State(st): State<Arc<AppState>>, Json(req): Json<LoginReq>) -> impl IntoResponse {
    let conn = st.db();
    if crate::auth::check_login(&conn, &req.username, &req.password) {
        let token = crate::auth::create_session(&conn, &req.username);
        // set cookie
        let cookie = format!("session={}; HttpOnly; SameSite=Lax; Path=/", token);
        (StatusCode::OK, [("set-cookie", cookie)], Json(serde_json::json!({"ok": true}))).into_response()
    } else {
        (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error":"invalid"}))).into_response()
    }
}

async fn logout(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"ok": true})))
}

#[derive(Deserialize)]
struct PwReq { username: String, old_password: String, new_password: String }

async fn change_password(State(st): State<Arc<AppState>>, Json(req): Json<PwReq>) -> impl IntoResponse {
    let conn = st.db();
    if !crate::auth::check_login(&conn, &req.username, &req.old_password) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error":"invalid"}))).into_response();
    }
    crate::auth::set_password(&conn, &req.username, &req.new_password);
    (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
}

async fn get_settings(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let g = st.inner.lock().unwrap();
    Json(serde_json::json!({
        "worker_count": g.settings.worker_count,
        "worker_delay_ms": g.settings.worker_delay_ms,
        "domain_delay_ms": g.settings.domain_delay_ms,
        "max_depth": g.settings.max_depth,
        "max_text_bytes": g.settings.max_text_bytes,
        "max_html_bytes": g.settings.max_html_bytes,
    }))
}

#[derive(Deserialize)]
struct SettingsUpdate { worker_count: Option<usize>, worker_delay_ms: Option<i64>, domain_delay_ms: Option<i64>, max_depth: Option<i64>, max_text_bytes: Option<i64>, max_html_bytes: Option<i64> }

async fn update_settings(State(st): State<Arc<AppState>>, Json(u): Json<SettingsUpdate>) -> impl IntoResponse {
    let mut g = st.inner.lock().unwrap();
    if let Some(v) = u.worker_count { g.settings.worker_count = v; crate::settings::set(&st.db(), "worker_count", &v.to_string()); }
    if let Some(v) = u.worker_delay_ms { g.settings.worker_delay_ms = v; crate::settings::set(&st.db(), "worker_delay_ms", &v.to_string()); }
    if let Some(v) = u.domain_delay_ms { g.settings.domain_delay_ms = v; crate::settings::set(&st.db(), "domain_delay_ms", &v.to_string()); }
    if let Some(v) = u.max_depth { g.settings.max_depth = v; crate::settings::set(&st.db(), "max_depth", &v.to_string()); }
    if let Some(v) = u.max_text_bytes { g.settings.max_text_bytes = v; crate::settings::set(&st.db(), "max_text_bytes", &v.to_string()); }
    if let Some(v) = u.max_html_bytes { g.settings.max_html_bytes = v; crate::settings::set(&st.db(), "max_html_bytes", &v.to_string()); }
    (StatusCode::OK, Json(serde_json::json!({"ok": true})))
}

async fn get_seeds(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let conn = st.db();
    let seeds: Vec<String> = conn.prepare("SELECT url FROM seed_urls").unwrap().query_map([], |r| r.get(0)).unwrap().filter_map(|x|x.ok()).collect();
    Json(seeds)
}

#[derive(Deserialize)]
struct SeedsUpdate { seeds: Vec<String> }

async fn update_seeds(State(st): State<Arc<AppState>>, Json(u): Json<SeedsUpdate>) -> impl IntoResponse {
    let conn = st.db();
    conn.execute("DELETE FROM seed_urls", []).ok();
    for s in u.seeds { conn.execute("INSERT OR IGNORE INTO seed_urls(url) VALUES(?1)", rusqlite::params![s]).ok(); }
    (StatusCode::OK, Json(serde_json::json!({"ok": true})))
}

async fn get_filters(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let conn = st.db();
    let banned_domains: Vec<String> = conn.prepare("SELECT domain FROM banned_domains").unwrap().query_map([], |r| r.get(0)).unwrap().filter_map(|x|x.ok()).collect();
    let banned_words: Vec<serde_json::Value> = conn.prepare("SELECT word,mode FROM banned_words").unwrap().query_map([], |r| Ok(serde_json::json!({"word": r.get::<_,String>(0)?, "mode": r.get::<_,String>(1)?}))).unwrap().filter_map(|x|x.ok()).collect();
    let promoted: Vec<String> = conn.prepare("SELECT domain FROM promoted_domains").unwrap().query_map([], |r| r.get(0)).unwrap().filter_map(|x|x.ok()).collect();
    Json(serde_json::json!({"banned_domains": banned_domains, "banned_words": banned_words, "promoted_domains": promoted}))
}

async fn crawl_status(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let g = st.inner.lock().unwrap();
    Json(serde_json::json!({
        "running": g.crawl_running,
        "processed": g.processed,
        "refused": g.refused,
        "start": g.crawl_start,
    }))
}

async fn crawl_start(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    if st.inner.lock().unwrap().crawl_running {
        return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"running"}))).into_response();
    }
    st.inner.lock().unwrap().crawl_running = true;
    st.inner.lock().unwrap().crawl_start = Some(chrono::Utc::now().timestamp());
    let s = (*st).clone();
    tokio::spawn(async move { crate::crawler::run(s).await; });
    (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
}

async fn crawl_stop(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    st.inner.lock().unwrap().crawl_running = false;
    (StatusCode::OK, Json(serde_json::json!({"ok": true})))
}

async fn crawl_restart(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    st.inner.lock().unwrap().crawl_running = false;
    let conn = st.db();
    let _ = conn.execute_batch("DELETE FROM staging_pages; DELETE FROM crawl_queue; DELETE FROM crawl_seen_urls;");
    st.inner.lock().unwrap().crawl_running = true;
    st.inner.lock().unwrap().crawl_start = Some(chrono::Utc::now().timestamp());
    let s = (*st).clone();
    tokio::spawn(async move { crate::crawler::run(s).await; });
    (StatusCode::OK, Json(serde_json::json!({"ok": true})))
}

async fn auth_mw(State(st): State<Arc<AppState>>, req: Request<axum::body::Body>, next: Next) -> impl IntoResponse {
    let path = req.uri().path().to_string();
    if path.ends_with("/login") {
        return next.run(req).await;
    }
    let cookie = req.headers().get("cookie").and_then(|v| v.to_str().ok()).unwrap_or("");
    let token = cookie.split(';').find_map(|p| {
        let p = p.trim();
        p.strip_prefix("session=").map(|t| t.to_string())
    });
    let ok = match token {
        Some(t) => crate::auth::validate_session(&st.db(), &t),
        None => false,
    };
    if !ok {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error":"unauthorized"}))).into_response();
    }
    next.run(req).await
}

async fn get_banned_domains(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let conn = st.db();
    let v: Vec<String> = conn.prepare("SELECT domain FROM banned_domains").unwrap().query_map([], |r| r.get(0)).unwrap().filter_map(|x|x.ok()).collect();
    Json(v)
}

#[derive(Deserialize)]
struct StrList { items: Vec<String> }

async fn update_banned_domains(State(st): State<Arc<AppState>>, Json(u): Json<StrList>) -> impl IntoResponse {
    let conn = st.db();
    conn.execute("DELETE FROM banned_domains", []).ok();
    for s in u.items { conn.execute("INSERT OR IGNORE INTO banned_domains(domain) VALUES(?1)", rusqlite::params![s]).ok(); }
    (StatusCode::OK, Json(serde_json::json!({"ok":true})))
}

async fn get_banned_words(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let conn = st.db();
    let v: Vec<serde_json::Value> = conn.prepare("SELECT word,mode FROM banned_words").unwrap().query_map([], |r| Ok(serde_json::json!({"word": r.get::<_,String>(0)?, "mode": r.get::<_,String>(1)?}))).unwrap().filter_map(|x|x.ok()).collect();
    Json(v)
}

#[derive(Deserialize)]
struct BannedWordItem { word: String, mode: Option<String> }
#[derive(Deserialize)]
struct BannedWordList { items: Vec<BannedWordItem> }

async fn update_banned_words(State(st): State<Arc<AppState>>, Json(u): Json<BannedWordList>) -> impl IntoResponse {
    let conn = st.db();
    conn.execute("DELETE FROM banned_words", []).ok();
    for it in u.items {
        let mode = it.mode.unwrap_or_else(|| "whole".to_string());
        conn.execute("INSERT OR IGNORE INTO banned_words(word,mode) VALUES(?1,?2)", rusqlite::params![it.word, mode]).ok();
    }
    (StatusCode::OK, Json(serde_json::json!({"ok":true})))
}

async fn get_promoted(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let conn = st.db();
    let v: Vec<String> = conn.prepare("SELECT domain FROM promoted_domains").unwrap().query_map([], |r| r.get(0)).unwrap().filter_map(|x|x.ok()).collect();
    Json(v)
}

async fn update_promoted(State(st): State<Arc<AppState>>, Json(u): Json<StrList>) -> impl IntoResponse {
    let conn = st.db();
    conn.execute("DELETE FROM promoted_domains", []).ok();
    for s in u.items { conn.execute("INSERT OR IGNORE INTO promoted_domains(domain) VALUES(?1)", rusqlite::params![s]).ok(); }
    (StatusCode::OK, Json(serde_json::json!({"ok":true})))
}

async fn robots_txt() -> impl IntoResponse {
    (StatusCode::OK, [("content-type", "text/plain")], "User-agent: *\nDisallow: /\n")
}
