use ssse::db;
use tempfile::tempdir;

#[test]
fn robots_root() {
    let rules = "User-agent: *\nDisallow: /";
    assert!(!ssse::crawler::robots_text_allowed_pub(rules, "/"));
}

#[test]
fn robots_disallow_all() {
    let rules = "User-agent: *\nDisallow: /";
    assert!(!ssse::crawler::robots_text_allowed_pub(rules, "/anything"));
}

#[test]
fn robots_allow_all_empty() {
    assert!(ssse::crawler::robots_text_allowed_pub("", "/"));
}

#[test]
fn robots_disallow_all_dup() {
    let rules = "User-agent: *\nDisallow: /";
    assert!(!ssse::crawler::robots_text_allowed_pub(rules, "/anything"));
}

#[test]
fn robots_empty_allows() {
    assert!(ssse::crawler::robots_text_allowed_pub("", "/anything"));
}

#[test]
fn robots_allow_disallow() {
    let rules = "User-agent: *\nDisallow: /private\nAllow: /private/public";
    assert!(!ssse::crawler::robots_text_allowed_pub(
        rules,
        "/private/secret"
    ));
    assert!(ssse::crawler::robots_text_allowed_pub(rules, "/public"));
}

#[test]
fn auth_hash_verify() {
    let h = ssse::auth::hash_password("secret");
    assert!(ssse::auth::verify_password(&h, "secret"));
    assert!(!ssse::auth::verify_password(&h, "wrong"));
}

#[test]
fn fts_phrase() {
    let q = ssse::api::build_fts_query("\"hello world\"");
    assert!(q.contains("\"hello world\""));
}

#[test]
fn fts_search() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("t.db");
    ssse::db::init(&db).unwrap();
    let conn = ssse::db::open(&db).unwrap();
    conn.execute("INSERT INTO pages(url,domain,title,description,text,language,published_at,crawled_at) VALUES('https://example.com','example.com','Hello World','desc','body text',NULL,NULL,1)", []).unwrap();
    conn.execute("INSERT INTO pages_fts(pages_fts) VALUES('rebuild')", [])
        .ok();
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM pages p JOIN pages_fts f ON f.rowid=p.id WHERE pages_fts MATCH 'hello'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

#[test]
fn db_schema_tables() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("t.db");
    ssse::db::init(&db).unwrap();
    let conn = ssse::db::open(&db).unwrap();
    for t in [
        "pages",
        "images",
        "videos",
        "news",
        "domains",
        "refused_urls",
        "settings",
        "seed_urls",
        "crawl_queue",
    ] {
        let n: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{}'",
                    t
                ),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "missing table {}", t);
    }
}

#[test]
fn db_init_and_settings() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("t.db");
    db::init(&db).unwrap();
    let conn = db::open(&db).unwrap();
    assert!(conn
        .execute("INSERT INTO settings(key,value) VALUES('a','b')", [])
        .is_ok());
}

#[test]
fn url_validation() {
    assert!(ssse::crawler::is_valid_https_url_pub("https://example.com"));
    assert!(!ssse::crawler::is_valid_https_url_pub("http://example.com"));
    assert!(!ssse::crawler::is_valid_https_url_pub("https://127.0.0.1"));
}

#[test]
fn psl_domain() {
    assert_eq!(psl::domain_str("en.wikipedia.org"), Some("wikipedia.org"));
    assert_eq!(psl::domain_str("example.co.uk"), Some("example.co.uk"));
    assert_ne!(psl::domain_str("a.co.uk"), psl::domain_str("b.co.uk"));
}

#[test]
fn db_migration_idempotent() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("t.db");
    ssse::db::init(&db).unwrap();
    ssse::db::init(&db).unwrap();
    let conn = ssse::db::open(&db).unwrap();
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn fts_query_operators() {
    assert_eq!(
        ssse::api::build_fts_query("hello world"),
        "\"hello\" AND \"world\""
    );
    assert!(ssse::api::build_fts_query("-spam hello").contains("NOT"));
    assert!(ssse::api::build_fts_query("site:example.com hello").contains("hello"));
}

#[test]
fn normalize_url_empty() {
    assert!(ssse::crawler::normalize_url("").is_none());
}

#[test]
fn normalize_url_extra() {
    assert_eq!(
        ssse::crawler::normalize_url("https://example.com/a?x=1#frag"),
        Some("https://example.com/a?x=1".to_string())
    );
}

#[test]
fn normalize_url() {
    assert_eq!(
        ssse::crawler::normalize_url("https://example.com/path#frag"),
        Some("https://example.com/path".to_string())
    );
    assert!(ssse::crawler::normalize_url("not a url").is_none());
}

#[test]
fn url_validation_ipv6() {
    assert!(!ssse::crawler::is_valid_https_url_pub("https://[::1]/"));
}

#[test]
fn url_validation_extra() {
    assert!(ssse::crawler::is_valid_https_url_pub("https://example.com"));
    assert!(!ssse::crawler::is_valid_https_url_pub("http://example.com"));
    assert!(!ssse::crawler::is_valid_https_url_pub("https://127.0.0.1"));
    assert!(!ssse::crawler::is_valid_https_url_pub("ftp://example.com"));
}

#[test]
fn banned_word_empty() {
    assert!(!ssse::crawler::banned_word_matches(
        "",
        "substring",
        "anything"
    ));
}

#[test]
fn banned_word_exact() {
    assert!(ssse::crawler::banned_word_matches("bad", "whole", "bad"));
}

#[test]
fn banned_word_punct() {
    assert!(ssse::crawler::banned_word_matches(
        "bad",
        "whole",
        "this is bad, here"
    ));
}

#[test]
fn banned_word_unicode() {
    assert!(ssse::crawler::banned_word_matches(
        "café",
        "substring",
        "J'ai un CAFÉ ici"
    ));
}

#[test]
fn banned_word_matching() {
    assert!(ssse::crawler::banned_word_matches(
        "bad",
        "substring",
        "this is badword"
    ));
    assert!(ssse::crawler::banned_word_matches(
        "bad",
        "whole",
        "this is bad here"
    ));
    assert!(!ssse::crawler::banned_word_matches(
        "bad",
        "whole",
        "badword here"
    ));
    assert!(ssse::crawler::banned_word_matches(
        "BAD",
        "substring",
        "this is badword"
    ));
}

#[test]
fn settings_default_depth() {
    let s = ssse::settings::Settings::default();
    assert_eq!(s.max_depth, 10);
}

#[test]
fn settings_default_interval() {
    let s = ssse::settings::Settings::default();
    assert_eq!(s.crawl_interval_days, 7);
}

#[test]
fn settings_default_workers() {
    let s = ssse::settings::Settings::default();
    assert!(s.worker_count >= 1);
    assert_eq!(s.max_text_bytes, 100 * 1024);
}

#[test]
fn settings_interval_roundtrip() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("t.db");
    ssse::db::init(&db).unwrap();
    let conn = ssse::db::open(&db).unwrap();
    ssse::settings::set(&conn, "crawl_interval_days", "14");
    let s = ssse::settings::load(&conn);
    assert_eq!(s.crawl_interval_days, 14);
}

#[test]
fn settings_roundtrip() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("t.db");
    ssse::db::init(&db).unwrap();
    let conn = ssse::db::open(&db).unwrap();
    ssse::settings::set(&conn, "worker_count", "4");
    let s = ssse::settings::load(&conn);
    assert_eq!(s.worker_count, 4);
}

#[test]
fn robots_allow_override() {
    let rules = "User-agent: *\nDisallow: /x\nAllow: /x/y";
    assert!(ssse::crawler::robots_text_allowed_pub(rules, "/x/y"));
    assert!(!ssse::crawler::robots_text_allowed_pub(rules, "/x/z"));
}

#[test]
fn normalize_url_ipv6() {
    // normalize should handle brackets
    let r = ssse::crawler::normalize_url("https://[::1]/path");
    assert!(r.is_some());
}
