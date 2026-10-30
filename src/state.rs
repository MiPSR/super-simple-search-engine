use crate::settings::Settings;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Clone)]
pub struct AppState {
    pub db_path: PathBuf,
    pub socket_path: PathBuf,
    pub bind: String,
    pub inner: Arc<Mutex<Inner>>,
    pub notify: Arc<Notify>,
}

pub struct Inner {
    pub settings: Settings,
    pub crawl_running: bool,
    pub crawl_start: Option<i64>,
    pub processed: u64,
    pub refused: u64,
    pub last_success: Option<i64>,
    pub last_duration_ms: Option<i64>,
}

impl AppState {
    pub fn new(db_path: PathBuf, socket_path: PathBuf, bind: String) -> Self {
        let conn = crate::db::open(&db_path).unwrap();
        let settings = crate::settings::load(&conn);
        drop(conn);
        Self {
            db_path,
            socket_path,
            bind,
            inner: Arc::new(Mutex::new(Inner {
                settings,
                crawl_running: false,
                crawl_start: None,
                processed: 0,
                refused: 0,
                last_success: None,
                last_duration_ms: None,
            })),
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn db(&self) -> Connection {
        crate::db::open(&self.db_path).unwrap()
    }

    pub fn spawn_socket_listener(&self) {
        let state = self.clone();
        let socket_path = self.socket_path.clone();
        tokio::spawn(async move {
            // remove existing
            let _ = std::fs::remove_file(&socket_path);
            if let Some(parent) = socket_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
            // restrict perms
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600));
            }
            loop {
                if let Ok((mut stream, _)) = listener.accept().await {
                    let st = state.clone();
                    tokio::spawn(async move {
                        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
                        let (r, mut w) = stream.split();
                        let mut reader = tokio::io::BufReader::new(r);
                        let mut buf = String::new();
                        use tokio::io::AsyncReadExt;
                        let _ = reader.read_to_string(&mut buf).await;
                        let lines: Vec<&str> = buf.lines().collect();
                        let cmd = lines.first().copied().unwrap_or("").trim();
                        let resp = handle_cmd(&st, cmd, &lines).await;
                        let _ = w.write_all(resp.as_bytes()).await;
                    });
                }
            }
        });
    }

    pub fn spawn_scheduler(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                if state.inner.lock().unwrap().crawl_running {
                    continue;
                }
                // check interval
                let last = state.inner.lock().unwrap().last_success;
                let interval_days = state.inner.lock().unwrap().settings.crawl_interval_days;
                let now = chrono::Utc::now().timestamp();
                let should = match last {
                    Some(ts) => (now - ts) > interval_days * 86400,
                    None => true,
                };
                if should {
                    // only auto-start if seed urls exist
                    let conn = state.db();
                    let seeds: i64 = conn
                        .query_row("SELECT COUNT(*) FROM seed_urls", [], |r| r.get(0))
                        .unwrap_or(0);
                    if seeds > 0 {
                        state.inner.lock().unwrap().crawl_running = true;
                        state.inner.lock().unwrap().crawl_start = Some(now);
                        let st = state.clone();
                        tokio::spawn(async move {
                            crate::crawler::run(st).await;
                        });
                    }
                }
            }
        });
    }
}

async fn handle_cmd(state: &AppState, cmd: &str, lines: &[&str]) -> String {
    match cmd {
        "crawl_start" => {
            if state.inner.lock().unwrap().crawl_running {
                return "already running\n".to_string();
            }
            state.inner.lock().unwrap().crawl_running = true;
            state.inner.lock().unwrap().crawl_start = Some(chrono::Utc::now().timestamp());
            let st = state.clone();
            tokio::spawn(async move {
                crate::crawler::run(st).await;
            });
            "started\n".to_string()
        }
        "crawl_stop" => {
            state.inner.lock().unwrap().crawl_running = false;
            "stopped\n".to_string()
        }
        "crawl_restart" => {
            state.inner.lock().unwrap().crawl_running = false;
            // discard staging
            let conn = state.db();
            let _ = conn.execute_batch("DELETE FROM staging_pages; DELETE FROM crawl_queue;");
            state.inner.lock().unwrap().crawl_running = true;
            state.inner.lock().unwrap().crawl_start = Some(chrono::Utc::now().timestamp());
            let st = state.clone();
            tokio::spawn(async move {
                crate::crawler::run(st).await;
            });
            "restarted\n".to_string()
        }
        "crawl_status" => {
            let g = state.inner.lock().unwrap();
            format!(
                "running: {}\nprocessed: {}\nrefused: {}\n",
                g.crawl_running, g.processed, g.refused
            )
        }
        "password_change" => {
            let username = lines.get(1).copied().unwrap_or("admin").trim();
            let password = lines.get(2).copied().unwrap_or("").trim();
            if password.is_empty() {
                return "password required\n".to_string();
            }
            let conn = state.db();
            crate::auth::set_password(&conn, username, password);
            "password changed\n".to_string()
        }
        _ => "unknown command\n".to_string(),
    }
}
