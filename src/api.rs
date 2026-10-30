use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::{Request, StatusCode, Uri},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};

type ApiResult = Result<Json<Value>, ApiError>;
pub struct ApiError(StatusCode, String);
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!(error=%e, "API operation failed");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "operation failed; see server logs".into(),
        )
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
fn invalid(message: impl ToString) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, message.to_string())
}

pub fn router(state: AppState) -> Router {
    let admin = Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/password", post(change_password))
        .route("/settings", get(get_settings).put(update_settings))
        .route("/seeds", get(get_seeds).put(update_seeds))
        .route("/filters", get(get_filters))
        .route(
            "/banned-domains",
            get(get_banned_domains).put(update_banned_domains),
        )
        .route(
            "/banned-words",
            get(get_banned_words).put(update_banned_words),
        )
        .route("/promoted", get(get_promoted).put(update_promoted))
        .route("/crawl/status", get(crawl_status))
        .route("/crawl/start", post(crawl_start))
        .route("/crawl/stop", post(crawl_stop))
        .route("/crawl/restart", post(crawl_restart))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_mw));
    Router::new()
        .route("/opensearch.xml", get(opensearch))
        .route(
            "/robots.txt",
            get(|| async { "User-agent: *\nDisallow: /\n" }),
        )
        .route("/api/search", get(search))
        .route("/api/stats", get(stats))
        .route("/api/stats/history", get(stats_history))
        .route("/api/stats/top", get(stats_top))
        .route("/api/stats/bottom", get(stats_bottom))
        .nest("/api/admin", admin)
        .fallback(get(asset))
        .layer(axum::middleware::from_fn(security_headers))
        .with_state(state)
}

async fn security_headers(req: Request<axum::body::Body>, next: Next) -> Response {
    let mut response = next.run(req).await;
    for (name,value) in [("x-robots-tag","noindex, nofollow"),("x-content-type-options","nosniff"),
        ("referrer-policy","no-referrer"),("x-frame-options","DENY"),
        ("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src https: data: blob:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'")] {
        response.headers_mut().insert(axum::http::HeaderName::from_static(name), axum::http::HeaderValue::from_static(value));
    }
    response
}

async fn asset(uri: Uri) -> Response {
    let (mime, data): (&str, &[u8]) = match uri.path() {
        "/" => (
            "text/html; charset=utf-8",
            include_bytes!("../static/index.html"),
        ),
        "/settings.html" => (
            "text/html; charset=utf-8",
            include_bytes!("../static/settings.html"),
        ),
        "/stats.html" => (
            "text/html; charset=utf-8",
            include_bytes!("../static/stats.html"),
        ),
        "/css/style.css" => ("text/css", include_bytes!("../static/css/style.css")),
        "/js/app.js" => ("text/javascript", include_bytes!("../static/js/app.js")),
        "/js/settings.js" => (
            "text/javascript",
            include_bytes!("../static/js/settings.js"),
        ),
        "/js/prefs.js" => ("text/javascript", include_bytes!("../static/js/prefs.js")),
        "/js/theme.js" => ("text/javascript", include_bytes!("../static/js/theme.js")),
        "/js/stats.js" => ("text/javascript", include_bytes!("../static/js/stats.js")),
        "/js/chart.js" => ("text/javascript", include_bytes!("../static/js/chart.js")),
        "/js/chart.LICENSE.txt" => (
            "text/plain; charset=utf-8",
            include_bytes!("../static/js/chart.LICENSE.txt"),
        ),
        "/opensearch.xml" => (
            "application/opensearchdescription+xml",
            include_bytes!("../static/opensearch.xml"),
        ),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    ([("content-type", mime)], data).into_response()
}

async fn opensearch(headers: axum::http::HeaderMap) -> Response {
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:3000");
    if host.parse::<axum::http::uri::Authority>().is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let scheme = if std::env::var("SSSE_INSECURE_COOKIE").as_deref() == Ok("1") {
        "http"
    } else {
        "https"
    };
    let origin = format!("{scheme}://{host}")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");
    let document = include_str!("../static/opensearch.xml")
        .replace("template=\"/?q=", &format!("template=\"{origin}/?q="));
    (
        [("content-type", "application/opensearchdescription+xml")],
        document,
    )
        .into_response()
}

#[derive(Default, Debug)]
struct ParsedQuery {
    positive: String,
    negative: String,
    site: String,
}
fn literal(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn parse_query(q: &str) -> ParsedQuery {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    for c in q.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if c.is_whitespace() && !quoted {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        } else {
            token.push(c);
        }
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    let mut parsed = ParsedQuery::default();
    let mut positive = Vec::new();
    let mut negative = Vec::new();
    for token in tokens {
        if let Some(site) = token.strip_prefix("site:") {
            parsed.site = site.trim_end_matches('.').to_lowercase();
        } else if let Some(term) = token.strip_prefix('-').filter(|s| !s.is_empty()) {
            negative.push(literal(term));
        } else {
            positive.push(literal(&token));
        }
    }
    parsed.positive = positive.join(" AND ");
    parsed.negative = negative.join(" OR ");
    parsed
}
pub fn build_fts_query(q: &str) -> String {
    let p = parse_query(q);
    if p.negative.is_empty() {
        p.positive
    } else if p.positive.is_empty() {
        String::new()
    } else {
        format!("{} NOT ({})", p.positive, p.negative)
    }
}

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
    category: Option<String>,
    page: Option<u32>,
    per_page: Option<u32>,
}
async fn search(State(st): State<AppState>, Query(p): Query<SearchParams>) -> ApiResult {
    let q = p.q.unwrap_or_default();
    if q.len() > 4096 {
        return Err(invalid("query exceeds 4096 bytes"));
    }
    let category = p.category.unwrap_or_else(|| "PAGES".into()).to_uppercase();
    let table = match category.as_str() {
        "PAGES" => "pages",
        "IMAGES" => "images",
        "VIDEOS" => "videos",
        "NEWS" => "news",
        _ => return Err(invalid("unknown search category")),
    };
    let page = p.page.unwrap_or(1).max(1);
    let per_page = p
        .per_page
        .unwrap_or(if table == "images" || table == "videos" {
            18
        } else {
            10
        });
    if !(1..=100).contains(&per_page) {
        return Err(invalid("per_page must be 1–100"));
    }
    let offset = i64::from(page - 1) * i64::from(per_page);
    let mut parsed = parse_query(&q);
    if !parsed.site.is_empty() {
        parsed.site = normalize_domain(&parsed.site).map_err(invalid)?;
    }
    if q.trim().is_empty() {
        return Ok(Json(
            json!({"results":[],"total":0,"page":page,"per_page":per_page}),
        ));
    }
    let data = st.database(move |conn| {
        conn.create_scalar_function("host_matches", 2, rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
            let url: String=ctx.get(0)?; let site: String=ctx.get(1)?;
            Ok(url::Url::parse(&url).ok().and_then(|u| u.host_str().map(|h| h==site || h.ends_with(&format!(".{site}")))).unwrap_or(false))
        })?;
        let fts = format!("{table}_fts");
        let filter = format!("(?1='' OR p.id IN (SELECT rowid FROM {fts} WHERE {fts} MATCH ?1))
            AND (?2='' OR p.id NOT IN (SELECT rowid FROM {fts} WHERE {fts} MATCH ?2))
            AND (?3='' OR host_matches(p.url,?3))");
        let tx = conn.transaction()?;
        let total: i64 = tx.query_row(&format!("SELECT count(*) FROM {table} p WHERE {filter}"), params![parsed.positive,parsed.negative,parsed.site], |r| r.get(0))?;
        let desc = if table=="images" {"p.alt"} else {"p.description"};
        let extra = if table=="videos" {"p.thumbnail_url"} else {"NULL"};
        // Logarithmic popularity can break near-relevance ties without overwhelming BM25.
        tx.create_scalar_function("popularity_boost",1,rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC, |ctx| Ok(1.0 + (ctx.get::<i64>(0)?.max(0) as f64).ln_1p().min(20.0)*0.01))?;
        let sql = format!("SELECT p.url,p.title,{desc},p.domain,p.published_at,{extra}
            FROM {table} p LEFT JOIN domains d ON d.registrable=p.domain WHERE {filter}
            ORDER BY EXISTS(SELECT 1 FROM promoted_domains pr WHERE pr.domain=p.domain) DESC,
            (CASE WHEN ?1='' THEN 0 ELSE (SELECT bm25({fts},5.0,2.0,1.0) FROM {fts} WHERE rowid=p.id AND {fts} MATCH ?1) END)*popularity_boost(COALESCE(d.popularity,0)),p.url
            LIMIT ?4 OFFSET ?5");
        let mut stmt=tx.prepare(&sql)?;
        let results=stmt.query_map(params![parsed.positive,parsed.negative,parsed.site,per_page,offset], |r| Ok(json!({
            "url":r.get::<_,String>(0)?,"title":r.get::<_,Option<String>>(1)?,"description":r.get::<_,Option<String>>(2)?,
            "domain":r.get::<_,Option<String>>(3)?,"published_at":r.get::<_,Option<i64>>(4)?,"extra":{"thumbnail":r.get::<_,Option<String>>(5)?}
        })))?.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt); tx.commit()?;
        Ok(json!({"results":results,"total":total,"page":page,"per_page":per_page}))
    }).await?;
    Ok(Json(data))
}

async fn stats(State(st): State<AppState>) -> ApiResult {
    Ok(Json(st.database(|conn| {
        let tx=conn.transaction()?;
        let snapshot = tx.query_row("SELECT completed_at,duration_ms,domains,pages,images,videos,news,refused,processed,details FROM crawl_history ORDER BY id DESC LIMIT 1", [], history_row).optional()?.unwrap_or_else(|| json!({"pages":0,"domains":0,"images":0,"videos":0,"news":0,"refused":0,"processed":0,"last_success":null,"last_duration_ms":null}));
        let mut result=snapshot;
        result["last_success"]=result["completed_at"].clone(); result["last_duration_ms"]=result["duration_ms"].clone();
        let mut stmt=tx.prepare("SELECT status,count(*) FROM http_responses GROUP BY status ORDER BY status")?;
        result["http_statuses"]=Value::Array(stmt.query_map([],|r| Ok(json!({"status":r.get::<_,Option<i64>>(0)?,"count":r.get::<_,i64>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?);
        drop(stmt);tx.commit()?;
        Ok(result)
    }).await?))
}
fn history_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let details =
        serde_json::from_str::<Value>(&r.get::<_, String>(9)?).unwrap_or_else(|_| json!({}));
    let mut row = json!({"completed_at":r.get::<_,i64>(0)?,"duration_ms":r.get::<_,i64>(1)?,"domains":r.get::<_,i64>(2)?,"pages":r.get::<_,i64>(3)?,"images":r.get::<_,i64>(4)?,"videos":r.get::<_,i64>(5)?,"news":r.get::<_,i64>(6)?,"refused":r.get::<_,i64>(7)?,"processed":r.get::<_,i64>(8)?});
    if let Some(details) = details.as_object() {
        for (key, value) in details {
            row[key] = value.clone();
        }
    }
    Ok(row)
}
async fn stats_history(State(st): State<AppState>) -> ApiResult {
    Ok(Json(st.database(|conn| { let mut stmt=conn.prepare("SELECT completed_at,duration_ms,domains,pages,images,videos,news,refused,processed,details FROM crawl_history ORDER BY id")?;
        let rows=stmt.query_map([],history_row)?.collect::<rusqlite::Result<Vec<_>>>()?; Ok(json!(rows)) }).await?))
}
async fn popularity(st: AppState, descending: bool) -> ApiResult {
    Ok(Json(st.database(move |conn| { let mut stmt=conn.prepare(&format!("SELECT registrable,popularity FROM domains d WHERE EXISTS(SELECT 1 FROM pages p WHERE p.domain=d.registrable) ORDER BY popularity {},registrable LIMIT 10",if descending {"DESC"} else {"ASC"}))?;
        let rows=stmt.query_map([], |r| Ok(json!({"domain":r.get::<_,String>(0)?,"popularity":r.get::<_,i64>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?; Ok(json!(rows)) }).await?))
}
async fn stats_top(State(st): State<AppState>) -> ApiResult {
    popularity(st, true).await
}
async fn stats_bottom(State(st): State<AppState>) -> ApiResult {
    popularity(st, false).await
}

fn session(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|c| c.trim().strip_prefix("session=").map(str::to_owned))
}
fn cookie(token: &str, clear: bool) -> String {
    let secure = if std::env::var("SSSE_INSECURE_COOKIE").as_deref() == Ok("1") {
        ""
    } else {
        "; Secure"
    };
    format!(
        "session={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}{}",
        if clear { 0 } else { 86400 },
        secure
    )
}
#[derive(Deserialize)]
struct LoginReq {
    username: String,
    password: String,
}
async fn login(
    State(st): State<AppState>,
    Json(req): Json<LoginReq>,
) -> Result<Response, ApiError> {
    if req.password.len() > 1024 || req.username.len() > 128 {
        return Err(invalid("credentials too long"));
    }
    let token = st
        .database(move |conn| {
            if !crate::auth::check_login(conn, &req.username, &req.password) {
                return Ok(None);
            }
            Ok(Some(crate::auth::create_session(conn, &req.username)?))
        })
        .await?;
    match token {
        Some(t) => Ok((
            [("set-cookie", cookie(&t, false))],
            Json(json!({"ok":true})),
        )
            .into_response()),
        None => Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "invalid credentials".into(),
        )),
    }
}
async fn logout(
    State(st): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(token) = session(&headers) {
        st.database(move |conn| {
            conn.execute("DELETE FROM sessions WHERE token=?1", [token])?;
            Ok(())
        })
        .await?;
    }
    Ok(([("set-cookie", cookie("", true))], Json(json!({"ok":true}))).into_response())
}
#[derive(Deserialize)]
struct PwReq {
    username: String,
    old_password: String,
    new_password: String,
}
async fn change_password(State(st): State<AppState>, Json(req): Json<PwReq>) -> ApiResult {
    if req.new_password.is_empty() || req.new_password.len() > 1024 || req.old_password.len() > 1024
    {
        return Err(invalid("password must contain 1–1024 bytes"));
    }
    let changed = st
        .database(move |conn| {
            if !crate::auth::check_login(conn, &req.username, &req.old_password) {
                return Ok(false);
            }
            crate::auth::set_password(conn, &req.username, &req.new_password)?;
            Ok(true)
        })
        .await?;
    if !changed {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "invalid credentials".into(),
        ));
    }
    Ok(Json(json!({"ok":true})))
}
async fn auth_mw(
    State(st): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD
    ) {
        let host = req
            .headers()
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let source = req
            .headers()
            .get("origin")
            .or_else(|| req.headers().get("referer"))
            .and_then(|v| v.to_str().ok());
        // Cookie-authenticated mutations must carry an exact same-origin browser signal.
        // Header-less local automation uses the Unix socket, not a CSRF exception.
        let valid = source
            .and_then(|s| url::Url::parse(s).ok())
            .is_some_and(|u| {
                matches!(u.scheme(), "http" | "https")
                    && u[url::Position::BeforeHost..url::Position::AfterPort] == *host
            });
        if !valid {
            return ApiError(
                StatusCode::FORBIDDEN,
                "same-origin Origin or Referer required".into(),
            )
            .into_response();
        }
    }
    if !req.uri().path().ends_with("/login") {
        let token = session(req.headers());
        let ok = st
            .database(
                move |conn| Ok(token.is_some_and(|t| crate::auth::validate_session(conn, &t))),
            )
            .await;
        if !matches!(ok, Ok(true)) {
            return ApiError(StatusCode::UNAUTHORIZED, "authentication required".into())
                .into_response();
        }
    }
    next.run(req).await
}

async fn get_settings(State(st): State<AppState>) -> ApiResult {
    Ok(Json(
        serde_json::to_value(&st.inner.lock().unwrap().settings).map_err(anyhow::Error::from)?,
    ))
}
async fn update_settings(State(st): State<AppState>, Json(value): Json<Value>) -> ApiResult {
    let mut merged =
        serde_json::to_value(&st.inner.lock().unwrap().settings).map_err(anyhow::Error::from)?;
    let obj = value
        .as_object()
        .ok_or_else(|| invalid("expected settings object"))?;
    for (key, value) in obj {
        if merged.get(key).is_none() {
            return Err(invalid(format!("unknown setting: {key}")));
        }
        merged[key] = value.clone();
    }
    let settings: crate::settings::Settings = serde_json::from_value(merged).map_err(invalid)?;
    settings.validate().map_err(invalid)?;
    let saved = settings.clone();
    st.database(move |conn| crate::settings::save(conn, &saved))
        .await?;
    st.inner.lock().unwrap().settings = settings;
    Ok(Json(json!({"ok":true,"applies":"next crawl"})))
}
pub fn normalize_domain(value: &str) -> anyhow::Result<String> {
    let s = value.trim().trim_end_matches('.');
    anyhow::ensure!(
        !s.is_empty()
            && !s.contains(['/', ':', '@', '?', '#', '*'])
            && !s.chars().any(char::is_whitespace),
        "invalid domain"
    );
    let u = url::Url::parse(&format!("https://{s}/"))?;
    anyhow::ensure!(
        matches!(u.host(), Some(url::Host::Domain(_))),
        "domain must be a hostname"
    );
    Ok(u.host_str().unwrap().to_owned())
}
async fn read_list(st: AppState, table: &'static str, column: &'static str) -> ApiResult {
    Ok(Json(
        st.database(move |conn| {
            let mut stmt =
                conn.prepare(&format!("SELECT {column} FROM {table} ORDER BY {column}"))?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(json!(rows))
        })
        .await?,
    ))
}
async fn replace_list(
    st: AppState,
    table: &'static str,
    column: &'static str,
    items: Vec<String>,
) -> ApiResult {
    st.database(move |conn| {
        let tx = conn.transaction()?;
        tx.execute(&format!("DELETE FROM {table}"), [])?;
        for item in items {
            tx.execute(
                &format!("INSERT OR IGNORE INTO {table}({column}) VALUES(?1)"),
                [item],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok(Json(json!({"ok":true})))
}
async fn get_seeds(State(st): State<AppState>) -> ApiResult {
    read_list(st, "seed_urls", "url").await
}
#[derive(Deserialize)]
struct SeedsUpdate {
    seeds: Vec<String>,
}
async fn update_seeds(State(st): State<AppState>, Json(u): Json<SeedsUpdate>) -> ApiResult {
    let mut urls = Vec::new();
    for s in u.seeds {
        let s = crate::crawler::normalize_url(s.trim())
            .filter(|s| crate::crawler::is_valid_https_url_pub(s))
            .ok_or_else(|| invalid("seeds must be HTTPS hostname URLs without credentials"))?;
        urls.push(s);
    }
    replace_list(st, "seed_urls", "url", urls).await
}
#[derive(Deserialize)]
struct StrList {
    items: Vec<String>,
}
async fn get_banned_domains(State(st): State<AppState>) -> ApiResult {
    read_list(st, "banned_domains", "domain").await
}
async fn update_banned_domains(State(st): State<AppState>, Json(u): Json<StrList>) -> ApiResult {
    let items = u
        .items
        .iter()
        .map(|s| normalize_domain(s))
        .collect::<anyhow::Result<Vec<_>>>()
        .map_err(invalid)?;
    replace_list(st, "banned_domains", "domain", items).await
}
async fn get_promoted(State(st): State<AppState>) -> ApiResult {
    read_list(st, "promoted_domains", "domain").await
}
async fn update_promoted(State(st): State<AppState>, Json(u): Json<StrList>) -> ApiResult {
    let mut items = Vec::new();
    for s in u.items {
        let domain = normalize_domain(&s).map_err(invalid)?;
        items.push(psl::domain_str(&domain).unwrap_or(&domain).to_owned());
    }
    replace_list(st, "promoted_domains", "domain", items).await
}
async fn get_banned_words(State(st): State<AppState>) -> ApiResult {
    Ok(Json(
        st.database(|conn| {
            let mut stmt = conn.prepare("SELECT word,mode FROM banned_words ORDER BY word")?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(json!({"word":r.get::<_,String>(0)?,"mode":r.get::<_,String>(1)?}))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(json!(rows))
        })
        .await?,
    ))
}
#[derive(Deserialize)]
struct WordItem {
    word: String,
    mode: String,
}
#[derive(Deserialize)]
struct WordList {
    items: Vec<WordItem>,
}
async fn update_banned_words(State(st): State<AppState>, Json(u): Json<WordList>) -> ApiResult {
    if u.items
        .iter()
        .any(|i| i.word.trim().is_empty() || !matches!(i.mode.as_str(), "whole" | "substring"))
    {
        return Err(invalid(
            "words must be nonempty; mode must be whole or substring",
        ));
    }
    st.database(move |conn| {
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM banned_words", [])?;
        for i in u.items {
            tx.execute(
                "INSERT OR IGNORE INTO banned_words(word,mode) VALUES(?1,?2)",
                params![i.word.trim(), i.mode],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok(Json(json!({"ok":true})))
}
async fn get_filters(State(st): State<AppState>) -> ApiResult {
    let domains = get_banned_domains(State(st.clone())).await?.0;
    let words = get_banned_words(State(st.clone())).await?.0;
    let promoted = get_promoted(State(st)).await?.0;
    Ok(Json(
        json!({"banned_domains":domains,"banned_words":words,"promoted_domains":promoted}),
    ))
}
async fn crawl_status(State(st): State<AppState>) -> ApiResult {
    Ok(Json(st.crawl_status().await?))
}
async fn crawl_start(State(st): State<AppState>) -> ApiResult {
    st.start_crawl(false)
        .await
        .map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
    Ok(Json(json!({"ok":true})))
}
async fn crawl_stop(State(st): State<AppState>) -> ApiResult {
    st.stop_crawl().await?;
    Ok(Json(json!({"ok":true})))
}
async fn crawl_restart(State(st): State<AppState>) -> ApiResult {
    st.start_crawl(true).await?;
    Ok(Json(json!({"ok":true})))
}
