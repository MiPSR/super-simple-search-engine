use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use ssse::{api, db, state::AppState};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

fn setup() -> (TempDir, AppState) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ssse.db");
    db::init(&path).unwrap();
    let state = AppState::new(path, dir.path().join("ssse.sock"), "127.0.0.1:3000".into());
    (dir, state)
}
async fn request(
    state: &AppState,
    method: &str,
    path: &str,
    body: Value,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "search.example")
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        req = req.header("cookie", cookie);
    }
    if let Some(origin) = origin {
        req = req.header("origin", origin);
    }
    let response = api::router(state.clone())
        .oneshot(
            req.body(if body.is_null() {
                Body::empty()
            } else {
                Body::from(body.to_string())
            })
            .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes).to_string()));
    (status, value, headers)
}
fn publish(state: &AppState) {
    let conn = state.db();
    for (url, domain, title) in [
        ("https://one.example/a", "one.example", "hello world"),
        ("https://two.example/b", "two.example", "hello spam"),
        ("https://sub.one.example/c", "one.example", "other thing"),
    ] {
        conn.execute(
            "INSERT INTO staging_pages(url,domain,title,text,crawled_at) VALUES(?1,?2,?3,'body',1)",
            rusqlite::params![url, domain, title],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO staging_images(url,domain,title,alt) VALUES(?1,?2,?3,'picture')",
            rusqlite::params![url, domain, title],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO staging_videos(url,domain,title,description) VALUES(?1,?2,?3,'video')",
            rusqlite::params![url, domain, title],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO staging_news(url,domain,title,description) VALUES(?1,?2,?3,'article')",
            rusqlite::params![url, domain, title],
        )
        .unwrap();
    }
    conn.execute("INSERT INTO staging_domains VALUES('one.example',2),('two.example',1),('unindexed.example',999)",[]).unwrap();
    ssse::crawler::finalize(state).unwrap();
}

#[tokio::test]
async fn all_categories_match_query_and_support_operators() {
    let (_dir, state) = setup();
    publish(&state);
    for category in ["PAGES", "IMAGES", "VIDEOS", "NEWS"] {
        for (q, count) in [
            ("hello", 2),
            ("absent", 0),
            ("hello%20-spam", 1),
            ("-spam", 2),
            ("site%3Aone.example", 2),
            ("hello%20site%3Aone.example", 1),
            ("%22hello%20world%22", 1),
            ("site%3Asub.one.example", 1),
        ] {
            let (status, data, _) = request(
                &state,
                "GET",
                &format!("/api/search?q={q}&category={category}"),
                Value::Null,
                None,
                None,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{category} {q}: {data}");
            assert_eq!(data["total"], count, "{category} {q}: {data}");
            assert_eq!(data["results"].as_array().unwrap().len(), count as usize);
        }
    }
}
#[tokio::test]
async fn malformed_queries_never_panic_and_pagination_is_bounded() {
    let (_dir, state) = setup();
    publish(&state);
    for q in [
        "hello-world",
        "%22unterminated",
        "%28",
        "NOT",
        "hello%20OR%20spam",
        "%2A",
        "%22%22",
    ] {
        let (status, _, _) = request(
            &state,
            "GET",
            &format!("/api/search?q={q}"),
            Value::Null,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{q}");
    }
    assert_eq!(
        request(
            &state,
            "GET",
            "/api/search?q=hello&per_page=4294967295",
            Value::Null,
            None,
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &state,
            "GET",
            "/api/search?q=hello&page=4294967295",
            Value::Null,
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}
#[tokio::test]
async fn promotions_do_not_duplicate_or_bypass_site_filters() {
    let (_dir, state) = setup();
    publish(&state);
    state
        .db()
        .execute("INSERT INTO promoted_domains VALUES('two.example')", [])
        .unwrap();
    let (_, first, _) = request(
        &state,
        "GET",
        "/api/search?q=hello&per_page=1",
        Value::Null,
        None,
        None,
    )
    .await;
    let (_, second, _) = request(
        &state,
        "GET",
        "/api/search?q=hello&per_page=1&page=2",
        Value::Null,
        None,
        None,
    )
    .await;
    assert_eq!(first["results"][0]["domain"], "two.example");
    assert_ne!(first["results"][0]["url"], second["results"][0]["url"]);
    let (_, data, _) = request(
        &state,
        "GET",
        "/api/search?q=hello%20site%3Aone.example",
        Value::Null,
        None,
        None,
    )
    .await;
    assert_eq!(data["total"], 1);
    assert_eq!(data["results"][0]["domain"], "one.example");
}
#[tokio::test]
async fn authentication_csrf_logout_and_validation() {
    let (_dir, state) = setup();
    ssse::auth::set_password(&mut state.db(), "admin", "secret").unwrap();
    let (status, _, headers) = request(
        &state,
        "POST",
        "/api/admin/login",
        json!({"username":"admin","password":"secret"}),
        None,
        Some("https://search.example"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = headers["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("Secure"));
    assert!(cookie.contains("HttpOnly"));
    let session = cookie.split(';').next().unwrap();
    assert_eq!(
        request(
            &state,
            "PUT",
            "/api/admin/settings",
            json!({"max_depth":-1}),
            Some(session),
            Some("https://search.example")
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &state,
            "PUT",
            "/api/admin/settings",
            json!({"max_depth":0,"max_html_bytes":0,"max_text_bytes":0}),
            Some(session),
            Some("https://search.example")
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &state,
            "PUT",
            "/api/admin/settings",
            json!({"max_depth":4}),
            Some(session),
            Some("https://search.example.attacker.test")
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &state,
            "PUT",
            "/api/admin/seeds",
            json!({"seeds":["https://valid.example/","http://invalid.example/"]}),
            Some(session),
            Some("https://search.example")
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (_, seeds, _) = request(
        &state,
        "GET",
        "/api/admin/seeds",
        Value::Null,
        Some(session),
        None,
    )
    .await;
    assert_eq!(seeds, json!([]));
    assert_eq!(
        request(
            &state,
            "POST",
            "/api/admin/logout",
            json!({}),
            Some(session),
            Some("https://search.example")
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &state,
            "GET",
            "/api/admin/settings",
            Value::Null,
            Some(session),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn publication_failure_rolls_back_everything() {
    let (_dir, state) = setup();
    publish(&state);
    let conn = state.db();
    conn.execute("CREATE TRIGGER fail_publication BEFORE INSERT ON images BEGIN SELECT RAISE(ABORT,'injected failure'); END",[]).unwrap();
    conn.execute("DELETE FROM staging_pages", []).unwrap();
    assert!(ssse::crawler::finalize(&state).is_err());
    assert_eq!(
        conn.query_row("SELECT count(*) FROM pages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM crawl_history", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let (_, data, _) = request(
        &state,
        "GET",
        "/api/search?q=hello",
        Value::Null,
        None,
        None,
    )
    .await;
    assert_eq!(data["total"], 2);
}
#[tokio::test]
async fn stopped_crawl_keeps_index_and_socket_commands_reply() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_dir, state) = setup();
    publish(&state);
    let listener = state.spawn_socket_listener().await.unwrap();
    let mut stream = tokio::net::UnixStream::connect(&state.socket_path)
        .await
        .unwrap();
    stream
        .write_all(b"{\"command\":\"crawl_status\",\"username\":null,\"password\":null}\n")
        .await
        .unwrap();
    let mut reply = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        stream.read_to_string(&mut reply),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&reply).unwrap()["ok"], true);
    state.start_crawl(false).await.unwrap();
    assert!(state.start_crawl(false).await.is_err());
    state.stop_crawl().await.unwrap();
    assert!(!state.inner.lock().unwrap().crawl_running);
    assert_eq!(
        state
            .db()
            .query_row("SELECT count(*) FROM pages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    listener.abort();
    state.shutdown().await.unwrap();
    assert!(state.start_crawl(false).await.is_err());
}
#[tokio::test]
async fn assets_are_embedded_and_stats_are_published_only() {
    let (_dir, state) = setup();
    publish(&state);
    state.inner.lock().unwrap().processed = 999;
    let (status, body, headers) = request(&state, "GET", "/", Value::Null, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.as_str().unwrap().contains("Super Simple"));
    assert_eq!(headers["x-robots-tag"], "noindex, nofollow");
    let (_, stats, _) = request(&state, "GET", "/api/stats", Value::Null, None, None).await;
    assert_eq!(stats["domains"], 2);
    assert_eq!(stats["processed"], 0);
    let restarted = AppState::new(
        state.db_path.clone(),
        state.socket_path.clone(),
        state.bind.clone(),
    );
    assert!(restarted.inner.lock().unwrap().last_success.is_some());
}

#[tokio::test]
async fn password_changes_revoke_all_sessions() {
    let (_dir, state) = setup();
    ssse::auth::set_password(&mut state.db(), "admin", "old").unwrap();
    let first = ssse::auth::create_session(&state.db(), "admin").unwrap();
    let second = ssse::auth::create_session(&state.db(), "admin").unwrap();
    let session = format!("session={first}");
    assert_eq!(
        request(
            &state,
            "POST",
            "/api/admin/password",
            json!({"username":"admin","old_password":"old","new_password":"new"}),
            Some(&session),
            Some("https://search.example")
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(!ssse::auth::validate_session(&state.db(), &first));
    assert!(!ssse::auth::validate_session(&state.db(), &second));
    assert!(ssse::auth::check_login(&state.db(), "admin", "new"));
    assert!(!ssse::auth::check_login(&state.db(), "admin", "old"));
}

#[test]
fn startup_cleanup_preserves_published_and_administrative_data() {
    let (_dir, state) = setup();
    publish(&state);
    ssse::auth::set_password(&mut state.db(), "admin", "secret").unwrap();
    ssse::settings::set(&state.db(), "max_depth", "0");
    state
        .db()
        .execute("INSERT INTO seed_urls VALUES('https://seed.example/')", [])
        .unwrap();
    db::init(&state.db_path).unwrap();
    let conn = state.db();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM pages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM staging_pages", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM crawl_history", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(ssse::settings::load(&conn).max_depth, 0);
    assert!(ssse::auth::check_login(&conn, "admin", "secret"));
    assert_eq!(
        conn.query_row("SELECT count(*) FROM seed_urls", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    conn.execute_batch("PRAGMA user_version=999").unwrap();
    assert!(db::migrate(&conn).is_err());
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        999
    );
}

#[tokio::test]
async fn searches_remain_consistent_during_republication() {
    let (_dir, state) = setup();
    publish(&state);
    let writer = state.clone();
    let task = tokio::task::spawn_blocking(move || {
        for generation in 0..16 {
            writer
                .db()
                .execute(
                    "UPDATE staging_pages SET title=?1 WHERE url='https://one.example/a'",
                    [if generation % 2 == 0 {
                        "changed"
                    } else {
                        "hello world"
                    }],
                )
                .unwrap();
            ssse::crawler::finalize(&writer).unwrap();
        }
    });
    for _ in 0..32 {
        let (status, data, _) = request(
            &state,
            "GET",
            "/api/search?q=hello",
            Value::Null,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            data["total"].as_u64().unwrap() as usize,
            data["results"].as_array().unwrap().len()
        );
    }
    task.await.unwrap();
}

#[test]
fn version_two_upgrade_preserves_settings_credentials_history_and_published_index() {
    let (_dir, state) = setup();
    publish(&state);
    ssse::settings::set(&state.db(), "max_depth", "0");
    ssse::auth::set_password(&mut state.db(), "admin", "upgrade-secret").unwrap();
    let conn = state.db();
    conn.execute_batch(
        "DROP INDEX crawl_queue_origin;
        ALTER TABLE crawl_queue DROP COLUMN parent_depth;
        ALTER TABLE crawl_queue DROP COLUMN origin;
        ALTER TABLE crawl_queue DROP COLUMN scope;
        ALTER TABLE crawl_queue DROP COLUMN kind;
        DROP TABLE crawl_aliases; DROP TABLE crawl_canonicals;
        PRAGMA user_version=2;",
    )
    .unwrap();
    conn.execute("INSERT INTO crawl_queue(url,depth,assigned,created_at) VALUES('https://unfinished.example/',1,1,1)",[]).unwrap();
    drop(conn);
    db::init(&state.db_path).unwrap();
    let conn = state.db();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM pages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM crawl_history", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM crawl_queue", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(ssse::settings::load(&conn).max_depth, 0);
    assert!(ssse::auth::check_login(&conn, "admin", "upgrade-secret"));
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM pages_fts WHERE pages_fts MATCH 'hello'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}
