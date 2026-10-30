use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub crawl_interval_days: u32,
    pub worker_count: usize,
    pub worker_delay_ms: u64,
    pub domain_delay_ms: u64,
    pub fifo_refill_ms: u64,
    pub max_depth: u32,
    pub max_text_bytes: usize,
    pub max_html_bytes: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            crawl_interval_days: 7,
            worker_count: std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1)),
            worker_delay_ms: 1000,
            domain_delay_ms: 1000,
            fifo_refill_ms: 1000,
            max_depth: 10,
            max_text_bytes: 100 * 1024,
            max_html_bytes: 1024 * 1024,
        }
    }
}

impl Settings {
    pub fn workers(&self) -> usize {
        if self.worker_count == 0 {
            Self::default().worker_count
        } else {
            self.worker_count
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.worker_count <= 4096,
            "worker_count must be between 0 and 4096 (0 = automatic)"
        );
        for delay in [
            self.worker_delay_ms,
            self.domain_delay_ms,
            self.fifo_refill_ms,
        ] {
            anyhow::ensure!(
                delay <= 86_400_000,
                "intervals must not exceed one day in milliseconds"
            );
        }
        Ok(())
    }
}

pub fn load(conn: &Connection) -> Settings {
    let mut values = serde_json::to_value(Settings::default()).expect("settings serialize");
    if let Ok(mut stmt) = conn.prepare("SELECT key,value FROM settings") {
        if let Ok(rows) =
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        {
            for (key, value) in rows.flatten() {
                if values.get(&key).is_some() {
                    if let Ok(n) = value.parse::<u64>() {
                        values[&key] = n.into();
                    }
                }
            }
        }
    }
    serde_json::from_value::<Settings>(values)
        .ok()
        .filter(|s| s.validate().is_ok())
        .unwrap_or_default()
}

pub fn save(conn: &mut Connection, settings: &Settings) -> anyhow::Result<()> {
    settings.validate()?;
    let tx = conn.transaction()?;
    for (key, value) in serde_json::to_value(settings)?
        .as_object()
        .expect("settings object")
    {
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", rusqlite::params![key, value.to_string()])?;
    }
    tx.commit()?;
    Ok(())
}

pub fn set(conn: &Connection, key: &str, value: &str) {
    conn.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", rusqlite::params![key, value]).expect("persist setting");
}
