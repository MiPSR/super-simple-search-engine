use crate::settings::Settings;
use rusqlite::Connection;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct AppState {
    pub db_path: PathBuf,
    pub socket_path: PathBuf,
    pub bind: String,
    pub inner: Arc<Mutex<Inner>>,
    control: Arc<AsyncMutex<Option<CrawlJob>>>,
    shutdown_token: CancellationToken,
}

struct CrawlJob {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

pub struct Inner {
    pub settings: Settings,
    pub crawl_running: bool,
    pub crawl_start: Option<i64>,
    pub processed: u64,
    pub refused: u64,
    pub active_workers: usize,
    pub crawl_worker_count: usize,
    pub last_success: Option<i64>,
    pub last_duration_ms: Option<i64>,
    pub status: String,
    pub error: Option<String>,
}

impl AppState {
    pub fn new(db_path: PathBuf, socket_path: PathBuf, bind: String) -> Self {
        let conn = crate::db::open(&db_path).expect("open initialized database");
        let settings = crate::settings::load(&conn);
        let last = conn
            .query_row(
                "SELECT completed_at,duration_ms FROM crawl_history ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        Self {
            db_path,
            socket_path,
            bind,
            control: Arc::new(AsyncMutex::new(None)),
            shutdown_token: CancellationToken::new(),
            inner: Arc::new(Mutex::new(Inner {
                settings,
                crawl_running: false,
                crawl_start: None,
                processed: 0,
                refused: 0,
                active_workers: 0,
                crawl_worker_count: 0,
                last_success: last.map(|x: (i64, i64)| x.0),
                last_duration_ms: last.map(|x| x.1),
                status: "idle".into(),
                error: None,
            })),
        }
    }

    pub fn db(&self) -> Connection {
        crate::db::open(&self.db_path).expect("open database")
    }

    pub async fn database<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> anyhow::Result<T> + Send + 'static,
    {
        let path = self.db_path.clone();
        tokio::task::spawn_blocking(move || f(&mut crate::db::open(&path)?)).await?
    }

    pub async fn start_crawl(&self, restart: bool) -> anyhow::Result<()> {
        let mut control = self.control.lock().await;
        anyhow::ensure!(
            !self.shutdown_token.is_cancelled(),
            "application is shutting down"
        );
        if let Some(job) = control.as_ref() {
            anyhow::ensure!(restart || job.task.is_finished(), "crawl already running");
        }
        if let Some(job) = control.take() {
            job.cancel.cancel();
            job.task.await?;
        }
        self.database(|conn| {
            crate::db::cleanup(conn)?;
            Ok(())
        })
        .await?;
        let settings = self.inner.lock().unwrap().settings.clone();
        settings.validate()?;
        {
            let mut g = self.inner.lock().unwrap();
            g.crawl_running = true;
            g.crawl_start = Some(chrono::Utc::now().timestamp());
            g.processed = 0;
            g.refused = 0;
            g.active_workers = 0;
            g.crawl_worker_count = settings.workers();
            g.status = "running".into();
            g.error = None;
        }
        let state = self.clone();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        // Synchronous SQLite/parsing work runs on a dedicated crawler runtime,
        // never on the HTTP runtime's executor threads.
        let task = tokio::spawn(async move {
            let s = state.clone();
            let result = tokio::task::spawn_blocking(move || {
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(
                        settings
                            .workers()
                            .min(std::thread::available_parallelism().map_or(2, usize::from)),
                    )
                    .enable_all()
                    .build()?
                    .block_on(crate::crawler::run(s, settings, token))
            })
            .await;
            let result = match result {
                Ok(r) => r,
                Err(e) => Err(e.into()),
            };
            if let Err(e) = &result {
                tracing::error!(error=%e, "crawl failed");
            }
            if let Err(e) = state
                .database(|conn| {
                    crate::settings::set(
                        conn,
                        "last_attempt",
                        &chrono::Utc::now().timestamp().to_string(),
                    );
                    crate::db::cleanup(conn)?;
                    conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")?;
                    Ok(())
                })
                .await
            {
                tracing::error!(error=%e, "crawl cleanup failed");
            }
            let mut g = state.inner.lock().unwrap();
            g.crawl_running = false;
            g.active_workers = 0;
            match result {
                Ok(true) => g.status = "completed".into(),
                Ok(false) => g.status = "stopped".into(),
                Err(e) => {
                    g.status = "failed".into();
                    g.error = Some(e.to_string());
                }
            }
        });
        *control = Some(CrawlJob { cancel, task });
        Ok(())
    }

    pub async fn stop_crawl(&self) -> anyhow::Result<()> {
        let mut control = self.control.lock().await;
        if let Some(job) = control.take() {
            job.cancel.cancel();
            job.task.await?;
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> anyhow::Result<()> {
        self.shutdown_token.cancel();
        self.stop_crawl().await
    }

    pub async fn crawl_status(&self) -> anyhow::Result<serde_json::Value> {
        let mut value = {
            let g = self.inner.lock().unwrap();
            serde_json::json!({"running":g.crawl_running,"state":g.status,"error":g.error,
                "start":g.crawl_start,"elapsed_seconds":g.crawl_start.map(|s| chrono::Utc::now().timestamp()-s),
                "processed":g.processed,"refused":g.refused,"worker_count":if g.crawl_running {g.crawl_worker_count} else {g.settings.workers()},
                "idle_workers":if g.crawl_running {g.crawl_worker_count.saturating_sub(g.active_workers)} else {0},
                "active_workers":g.active_workers,"last_success":g.last_success,"last_duration_ms":g.last_duration_ms})
        };
        let (pending, assigned) = self
            .database(|conn| {
                Ok((
                    conn.query_row(
                        "SELECT count(*) FROM crawl_queue WHERE assigned=0",
                        [],
                        |r| r.get::<_, i64>(0),
                    )?,
                    conn.query_row(
                        "SELECT count(*) FROM crawl_queue WHERE assigned=1",
                        [],
                        |r| r.get::<_, i64>(0),
                    )?,
                ))
            })
            .await?;
        value["queued"] = pending.into();
        value["assigned"] = assigned.into();
        value["fifo"] = (assigned - value["active_workers"].as_i64().unwrap_or(0))
            .max(0)
            .into();
        Ok(value)
    }

    pub async fn spawn_socket_listener(&self) -> anyhow::Result<tokio::task::JoinHandle<()>> {
        let path = &self.socket_path;
        if path.exists() {
            anyhow::ensure!(
                tokio::net::UnixStream::connect(path).await.is_err(),
                "another instance owns the control socket"
            );
            std::fs::remove_file(path)?;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let listener = tokio::net::UnixListener::bind(path)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let state = self.clone();
        Ok(tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {_=state.shutdown_token.cancelled()=>break, accepted=listener.accept()=>accepted};
                let Ok((mut stream, _)) = accepted else { break };
                let state = state.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
                    let mut line = String::new();
                    let read = tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        tokio::io::BufReader::new(&mut stream)
                            .take(16 * 1024)
                            .read_line(&mut line),
                    )
                    .await;
                    if !matches!(read, Ok(Ok(n)) if n > 0) {
                        return;
                    }
                    let result = match serde_json::from_str::<ControlRequest>(&line) {
                        Ok(req) => handle_cmd(&state, req).await,
                        Err(e) => Err(e.into()),
                    };
                    let response = match result {
                        Ok(v) => serde_json::json!({"ok":true,"data":v}),
                        Err(e) => serde_json::json!({"ok":false,"error":e.to_string()}),
                    };
                    let _ = stream.write_all(format!("{response}\n").as_bytes()).await;
                });
            }
        }))
    }

    pub fn spawn_scheduler(&self) -> tokio::task::JoinHandle<()> {
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {_=state.shutdown_token.cancelled()=>break,_=tokio::time::sleep(std::time::Duration::from_secs(60))=>{}}
                let (running, days, last) = {
                    let g = state.inner.lock().unwrap();
                    (
                        g.crawl_running,
                        g.settings.crawl_interval_days,
                        g.last_success,
                    )
                };
                if running || days == 0 {
                    continue;
                }
                let due = state
                    .database(move |conn| {
                        let attempt: i64 = conn
                            .query_row(
                                "SELECT value FROM settings WHERE key='last_attempt'",
                                [],
                                |r| r.get::<_, String>(0),
                            )
                            .ok()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        let seeds: i64 =
                            conn.query_row("SELECT count(*) FROM seed_urls", [], |r| r.get(0))?;
                        Ok(seeds > 0
                            && chrono::Utc::now().timestamp() - last.unwrap_or(0).max(attempt)
                                >= i64::from(days) * 86400)
                    })
                    .await;
                if matches!(due, Ok(true)) {
                    if let Err(e) = state.start_crawl(false).await {
                        tracing::warn!(error=%e,"scheduled crawl not started");
                    }
                }
            }
        })
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ControlRequest {
    pub command: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

async fn handle_cmd(state: &AppState, req: ControlRequest) -> anyhow::Result<serde_json::Value> {
    match req.command.as_str() {
        "crawl_start" => state.start_crawl(false).await?,
        "crawl_restart" => state.start_crawl(true).await?,
        "crawl_stop" => state.stop_crawl().await?,
        "crawl_status" => return state.crawl_status().await,
        "password_change" => {
            state
                .database(move |conn| {
                    crate::auth::set_password(
                        conn,
                        req.username.as_deref().unwrap_or("admin"),
                        req.password.as_deref().unwrap_or(""),
                    )
                })
                .await?;
        }
        _ => anyhow::bail!("unknown command"),
    }
    Ok(serde_json::json!("done"))
}
