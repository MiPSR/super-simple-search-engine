use ssse::db;
use tempfile::tempdir;

#[test]
fn db_init_and_settings() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("t.db");
    db::init(&db).unwrap();
    let conn = db::open(&db).unwrap();
    assert!(conn.execute("INSERT INTO settings(key,value) VALUES('a','b')", []).is_ok());
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
}
