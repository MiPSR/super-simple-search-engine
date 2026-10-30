use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::info;

use ssse::{api, db, state};

#[derive(Parser)]
#[command(name = "ssse", version, about = "Super Simple Search Engine")]
struct Cli {
    #[arg(
        long,
        global = true,
        env = "SSSE_DATA_DIR",
        default_value = "./ssse-data"
    )]
    data_dir: PathBuf,

    #[arg(
        long,
        global = true,
        env = "SSSE_BIND",
        default_value = "127.0.0.1:3000"
    )]
    bind: String,

    #[arg(long, global = true, env = "SSSE_SOCKET")]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Serve,
    Password {
        #[command(subcommand)]
        action: PasswordAction,
    },
    Crawl {
        #[command(subcommand)]
        action: CrawlAction,
    },
}

#[derive(Subcommand)]
enum PasswordAction {
    Change {
        #[arg(long)]
        username: Option<String>,
    },
}

#[derive(Subcommand)]
enum CrawlAction {
    Start,
    Stop,
    Restart,
    Status,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let socket = cli
        .socket
        .clone()
        .unwrap_or_else(|| cli.data_dir.join("ssse.sock"));

    match cli.cmd {
        None | Some(Commands::Serve) => {
            let new_directory = !cli.data_dir.exists();
            std::fs::create_dir_all(&cli.data_dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if new_directory {
                    std::fs::set_permissions(
                        &cli.data_dir,
                        std::fs::Permissions::from_mode(0o700),
                    )?;
                }
            }
            let instance_lock = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(cli.data_dir.join("ssse.lock"))?;
            instance_lock.try_lock().map_err(|e| {
                anyhow::anyhow!("another instance is using this data directory: {e}")
            })?;
            let db_path = cli.data_dir.join("ssse.db");
            db::init(&db_path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o600))?;
            }
            let state = state::AppState::new(db_path.clone(), socket.clone(), cli.bind.clone());
            let socket_task = state.spawn_socket_listener().await?;
            let scheduler = state.spawn_scheduler();
            info!("starting server on {}", cli.bind);
            let app = api::router(state.clone());
            let listener = tokio::net::TcpListener::bind(&cli.bind).await?;
            let state_sd = state.clone();
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let ctrl_c = tokio::signal::ctrl_c();
                    #[cfg(unix)]
                    {
                        let mut term = tokio::signal::unix::signal(
                            tokio::signal::unix::SignalKind::terminate(),
                        )
                        .unwrap();
                        tokio::select! {
                            _ = ctrl_c => {},
                            _ = term.recv() => {},
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = ctrl_c.await;
                    }
                    if let Err(e) = state_sd.shutdown().await {
                        tracing::error!(error=%e,"shutdown failed");
                    }
                })
                .await?;
            scheduler.abort();
            socket_task.abort();
            let _ = std::fs::remove_file(&socket);
            return Ok(());
        }
        _ => {}
    }

    // CLI commands (no DB init)
    match cli.cmd {
        Some(Commands::Password { action }) => match action {
            PasswordAction::Change { username } => {
                let user = username.unwrap_or_else(|| "admin".to_string());
                let pw = rpassword::prompt_password("New password: ")?;
                let confirm = rpassword::prompt_password("Confirm password: ")?;
                anyhow::ensure!(pw == confirm, "passwords do not match");
                control_call(
                    &socket,
                    state::ControlRequest {
                        command: "password_change".into(),
                        username: Some(user),
                        password: Some(pw),
                    },
                )
                .await?;
            }
        },
        Some(Commands::Crawl { action }) => {
            let msg = match action {
                CrawlAction::Start => "crawl_start",
                CrawlAction::Stop => "crawl_stop",
                CrawlAction::Restart => "crawl_restart",
                CrawlAction::Status => "crawl_status",
            };
            cli_call(&socket, msg).await?;
        }
        _ => {}
    }
    Ok(())
}

async fn cli_call(socket: &PathBuf, cmd: &str) -> anyhow::Result<()> {
    control_call(
        socket,
        state::ControlRequest {
            command: cmd.into(),
            username: None,
            password: None,
        },
    )
    .await
}

async fn control_call(socket: &PathBuf, req: state::ControlRequest) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;
    let mut stream = UnixStream::connect(socket).await?;
    stream
        .write_all(format!("{}\n", serde_json::to_string(&req)?).as_bytes())
        .await?;
    stream.shutdown().await?;
    let mut buf = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(120),
        stream.read_to_string(&mut buf),
    )
    .await??;
    let response: serde_json::Value = serde_json::from_str(&buf)?;
    anyhow::ensure!(
        response["ok"] == true,
        "{}",
        response["error"]
            .as_str()
            .unwrap_or("control command failed")
    );
    println!("{}", serde_json::to_string_pretty(&response["data"])?);
    Ok(())
}
