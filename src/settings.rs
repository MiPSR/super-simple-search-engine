use rusqlite::Connection;
use std::collections::HashMap;

#[derive(Clone)]
pub struct Settings {
    pub crawl_interval_days: i64,
    pub worker_count: usize,
    pub worker_delay_ms: i64,
    pub domain_delay_ms: i64,
    pub fifo_refill_ms: i64,
    pub max_depth: i64,
    pub max_text_bytes: i64,
    pub max_html_bytes: i64,
}

impl Default for Settings {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        Self {
            crawl_interval_days: 7,
            worker_count: (cpus / 2).max(1),
            worker_delay_ms: 1000,
            domain_delay_ms: 1000,
            fifo_refill_ms: 1000,
            max_depth: 10,
            max_text_bytes: 100 * 1024,
            max_html_bytes: 1024 * 1024,
        }
    }
}

pub fn load(conn: &Connection) -> Settings {
    let mut s = Settings::default();
    let mut stmt = conn.prepare("SELECT key,value FROM settings").unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .unwrap();
    let mut m: HashMap<String, String> = HashMap::new();
    for (k, v) in rows.flatten() {
        m.insert(k, v);
    }
    if let Some(v) = m.get("crawl_interval_days") {
        s.crawl_interval_days = v.parse().unwrap_or(s.crawl_interval_days);
    }
    if let Some(v) = m.get("worker_count") {
        s.worker_count = v.parse().unwrap_or(s.worker_count);
    }
    if let Some(v) = m.get("worker_delay_ms") {
        s.worker_delay_ms = v.parse().unwrap_or(s.worker_delay_ms);
    }
    if let Some(v) = m.get("domain_delay_ms") {
        s.domain_delay_ms = v.parse().unwrap_or(s.domain_delay_ms);
    }
    if let Some(v) = m.get("fifo_refill_ms") {
        s.fifo_refill_ms = v.parse().unwrap_or(s.fifo_refill_ms);
    }
    if let Some(v) = m.get("max_depth") {
        s.max_depth = v.parse().unwrap_or(s.max_depth);
    }
    if let Some(v) = m.get("max_text_bytes") {
        s.max_text_bytes = v.parse().unwrap_or(s.max_text_bytes);
    }
    if let Some(v) = m.get("max_html_bytes") {
        s.max_html_bytes = v.parse().unwrap_or(s.max_html_bytes);
    }
    s
}

pub fn set(conn: &Connection, key: &str, value: &str) {
    conn.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", rusqlite::params![key, value]).ok();
}
