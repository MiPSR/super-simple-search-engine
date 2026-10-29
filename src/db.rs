use rusqlite::{Connection, Result};
use std::path::Path;

pub fn init(path: &Path) -> Result<()> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
    migrate(&conn)?;
    // discard incomplete staging/temp crawl data from interrupted runs
    conn.execute_batch("DELETE FROM staging_pages; DELETE FROM staging_images; DELETE FROM staging_videos; DELETE FROM staging_news; DELETE FROM staging_domains; DELETE FROM staging_refused_urls; DELETE FROM staging_http_responses; DELETE FROM staging_robots; DELETE FROM crawl_queue; DELETE FROM crawl_seen_urls; DELETE FROM crawl_domains; DELETE FROM crawl_robots; DELETE FROM crawl_responses; DELETE FROM crawl_refused_urls; DELETE FROM crawl_links;").ok();
    Ok(())
}

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    Ok(conn)
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS credentials (
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL,
    password_hash TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
    token TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS seed_urls (url TEXT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS banned_domains (domain TEXT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS banned_words (word TEXT PRIMARY KEY, mode TEXT NOT NULL DEFAULT 'whole');
CREATE TABLE IF NOT EXISTS promoted_domains (domain TEXT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS crawl_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    completed_at INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,
    domains INTEGER NOT NULL,
    pages INTEGER NOT NULL,
    images INTEGER NOT NULL,
    videos INTEGER NOT NULL,
    news INTEGER NOT NULL,
    refused INTEGER NOT NULL,
    processed INTEGER NOT NULL
);

-- published index
CREATE TABLE IF NOT EXISTS pages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL UNIQUE,
    domain TEXT NOT NULL,
    title TEXT,
    description TEXT,
    text TEXT,
    language TEXT,
    published_at INTEGER,
    crawled_at INTEGER NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(title, description, text, content='pages', content_rowid='id');
CREATE TRIGGER IF NOT EXISTS pages_ai AFTER INSERT ON pages BEGIN
  INSERT INTO pages_fts(rowid, title, description, text) VALUES (new.id, new.title, new.description, new.text);
END;
CREATE TRIGGER IF NOT EXISTS pages_ad AFTER DELETE ON pages BEGIN
  INSERT INTO pages_fts(pages_fts, rowid, title, description, text) VALUES('delete', old.id, old.title, old.description, old.text);
END;

CREATE TABLE IF NOT EXISTS images (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL UNIQUE,
    referring_url TEXT,
    title TEXT,
    alt TEXT,
    domain TEXT,
    published_at INTEGER
);
CREATE TABLE IF NOT EXISTS videos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL UNIQUE,
    title TEXT,
    domain TEXT,
    thumbnail_url TEXT,
    published_at INTEGER,
    description TEXT
);
CREATE TABLE IF NOT EXISTS news (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL UNIQUE,
    title TEXT,
    domain TEXT,
    published_at INTEGER,
    description TEXT
);
CREATE TABLE IF NOT EXISTS domains (
    registrable TEXT PRIMARY KEY,
    popularity INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS refused_urls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL,
    domain TEXT,
    reason TEXT NOT NULL,
    status INTEGER,
    ts INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS http_responses (
    url TEXT PRIMARY KEY,
    status INTEGER,
    error TEXT
);
CREATE TABLE IF NOT EXISTS robots (
    origin TEXT PRIMARY KEY,
    url TEXT,
    status INTEGER,
    rules TEXT,
    crawl_delay REAL,
    fetched_at INTEGER
);

-- temporary crawl tables
CREATE TABLE IF NOT EXISTS crawl_queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL,
    depth INTEGER NOT NULL DEFAULT 0,
    assigned INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS crawl_seen_urls (url TEXT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS crawl_domains (registrable TEXT PRIMARY KEY, popularity INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS crawl_robots (
    origin TEXT PRIMARY KEY,
    url TEXT,
    status INTEGER,
    rules TEXT,
    crawl_delay REAL,
    fetched_at INTEGER
);
CREATE TABLE IF NOT EXISTS crawl_responses (
    url TEXT PRIMARY KEY,
    status INTEGER,
    error TEXT
);
CREATE TABLE IF NOT EXISTS crawl_refused_urls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL,
    domain TEXT,
    reason TEXT NOT NULL,
    status INTEGER,
    ts INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS crawl_links (
    src TEXT,
    dst TEXT NOT NULL
);
-- staging index tables (mirror published)
CREATE TABLE IF NOT EXISTS staging_pages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL UNIQUE,
    domain TEXT NOT NULL,
    title TEXT,
    description TEXT,
    text TEXT,
    language TEXT,
    published_at INTEGER,
    crawled_at INTEGER NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS staging_pages_fts USING fts5(title, description, text, content='staging_pages', content_rowid='id');
CREATE TRIGGER IF NOT EXISTS staging_pages_ai AFTER INSERT ON staging_pages BEGIN
  INSERT INTO staging_pages_fts(rowid, title, description, text) VALUES (new.id, new.title, new.description, new.text);
END;
CREATE TABLE IF NOT EXISTS staging_images (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL UNIQUE,
    referring_url TEXT,
    title TEXT,
    alt TEXT,
    domain TEXT,
    published_at INTEGER
);
CREATE TABLE IF NOT EXISTS staging_videos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL UNIQUE,
    title TEXT,
    domain TEXT,
    thumbnail_url TEXT,
    published_at INTEGER,
    description TEXT
);
CREATE TABLE IF NOT EXISTS staging_news (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL UNIQUE,
    title TEXT,
    domain TEXT,
    published_at INTEGER,
    description TEXT
);
CREATE TABLE IF NOT EXISTS staging_domains (
    registrable TEXT PRIMARY KEY,
    popularity INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS staging_refused_urls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL,
    domain TEXT,
    reason TEXT NOT NULL,
    status INTEGER,
    ts INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS staging_http_responses (
    url TEXT PRIMARY KEY,
    status INTEGER,
    error TEXT
);
CREATE TABLE IF NOT EXISTS staging_robots (
    origin TEXT PRIMARY KEY,
    url TEXT,
    status INTEGER,
    rules TEXT,
    crawl_delay REAL,
    fetched_at INTEGER
);
"#;
