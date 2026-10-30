use rusqlite::{Connection, Result};
use std::path::Path;

pub fn init(path: &Path) -> Result<()> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(15))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
    migrate(&conn)?;
    // discard incomplete staging/temp crawl data from interrupted runs
    cleanup(&conn)?;
    Ok(())
}

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(15))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    Ok(conn)
}

pub fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > 3 {
        return Err(rusqlite::Error::InvalidParameterName(
            "database schema is newer than this application; refusing to downgrade".into(),
        ));
    }
    if version == 3 {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    let conn = &tx;
    conn.execute_batch(SCHEMA)?;
    // Version 1 databases already contain these tables; add indexes/triggers without
    // replacing administrative settings or historical crawl snapshots.
    conn.execute_batch("DROP TRIGGER IF EXISTS staging_pages_ai; DROP TABLE IF EXISTS staging_pages_fts;
        CREATE INDEX IF NOT EXISTS crawl_queue_pending ON crawl_queue(assigned,id);
        DELETE FROM crawl_queue WHERE id NOT IN (SELECT min(id) FROM crawl_queue GROUP BY url);
        CREATE UNIQUE INDEX IF NOT EXISTS crawl_queue_url ON crawl_queue(url);
        CREATE INDEX IF NOT EXISTS pages_domain ON pages(domain);
        CREATE TRIGGER IF NOT EXISTS pages_au AFTER UPDATE ON pages BEGIN
          INSERT INTO pages_fts(pages_fts,rowid,title,description,text) VALUES('delete',old.id,old.title,old.description,old.text);
          INSERT INTO pages_fts(rowid,title,description,text) VALUES(new.id,new.title,new.description,new.text);
        END;
        CREATE VIRTUAL TABLE IF NOT EXISTS images_fts USING fts5(title,alt,content='images',content_rowid='id');
        CREATE VIRTUAL TABLE IF NOT EXISTS videos_fts USING fts5(title,description,content='videos',content_rowid='id');
        CREATE VIRTUAL TABLE IF NOT EXISTS news_fts USING fts5(title,description,content='news',content_rowid='id');
        INSERT INTO pages_fts(pages_fts) VALUES('rebuild');
        INSERT INTO images_fts(images_fts) VALUES('rebuild');
        INSERT INTO videos_fts(videos_fts) VALUES('rebuild');
        INSERT INTO news_fts(news_fts) VALUES('rebuild');
        PRAGMA user_version=1;")?;
    for table in ["http_responses", "staging_http_responses"] {
        add_column(conn, table, "duration_ms", "INTEGER")?;
    }
    add_column(
        conn,
        "crawl_history",
        "details",
        "TEXT NOT NULL DEFAULT '{}'",
    )?;
    add_column(conn, "crawl_queue", "kind", "TEXT NOT NULL DEFAULT 'page'")?;
    add_column(conn, "crawl_queue", "scope", "TEXT NOT NULL DEFAULT ''")?;
    add_column(conn, "crawl_queue", "origin", "TEXT NOT NULL DEFAULT ''")?;
    add_column(conn, "crawl_queue", "parent_depth", "INTEGER")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS crawl_canonicals(source TEXT PRIMARY KEY,target TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS crawl_aliases(alias TEXT PRIMARY KEY,target TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS crawl_queue_origin ON crawl_queue(origin);
        PRAGMA user_version=3;",
    )?;
    tx.commit()
}

fn add_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let columns = conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>>>()?;
    if !columns.iter().any(|c| c == column) {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

pub fn cleanup(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        "
        DELETE FROM staging_pages; DELETE FROM staging_images; DELETE FROM staging_videos;
        DELETE FROM staging_news; DELETE FROM staging_domains; DELETE FROM staging_refused_urls;
        DELETE FROM staging_http_responses; DELETE FROM staging_robots; DELETE FROM crawl_queue;
        DELETE FROM crawl_seen_urls; DELETE FROM crawl_domains; DELETE FROM crawl_robots;
        DELETE FROM crawl_responses; DELETE FROM crawl_refused_urls; DELETE FROM crawl_links;
        DELETE FROM crawl_canonicals; DELETE FROM crawl_aliases;",
    )?;
    tx.commit()
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
