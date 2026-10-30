use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::info;

use ssse::{api, db, state};

#[derive(Parser)]
#[command(name = "ssse", version, about = "Super Simple Search Engine")]
struct Cli {
    #[arg(long, env = "SSSE_DATA_DIR", default_value = "/var/lib/ssse")]
    data_dir: PathBuf,

    #[arg(long, env = "SSSE_BIND", default_value = "127.0.0.1:3000")]
    bind: String,

    #[arg(long, env = "SSSE_SOCKET", default_value = "/run/ssse/ssse.sock")]
    socket: PathBuf,

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

    match cli.cmd {
        None | Some(Commands::Serve) => {
            std::fs::create_dir_all(&cli.data_dir)?;
            let db_path = cli.data_dir.join("ssse.db");
            db::init(&db_path)?;
            let state = state::AppState::new(db_path.clone(), cli.socket.clone(), cli.bind.clone());
            state.spawn_socket_listener();
            state.spawn_scheduler();
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
                    state_sd.inner.lock().unwrap().crawl_running = false;
                })
                .await?;
            return Ok(());
        }
        _ => {}
    }

    // CLI commands (no DB init)
    match cli.cmd {
        Some(Commands::Password { action }) => {
            match action {
                PasswordAction::Change { username } => {
                    let user = username.unwrap_or_else(|| "admin".to_string());
                    // read password securely
                    eprint!("New password: ");
                    // disable echo
                    let _ = std::process::Command::new("stty").arg("-echo").status();
                    let mut pw = String::new();
                    std::io::stdin().read_line(&mut pw).ok();
                    let _ = std::process::Command::new("stty").arg("echo").status();
                    eprintln!();
                    let pw = pw.trim().to_string();
                    // send to socket
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    use tokio::net::UnixStream;
                    let mut stream = UnixStream::connect(&cli.socket).await?;
                    stream
                        .write_all(format!("password_change\n{}\n{}\n", user, pw).as_bytes())
                        .await?;
                    stream.shutdown().await.ok();
                    let mut buf = String::new();
                    stream.read_to_string(&mut buf).await?;
                    print!("{}", buf);
                }
            }
        }
        Some(Commands::Crawl { action }) => {
            let msg = match action {
                CrawlAction::Start => "crawl_start",
                CrawlAction::Stop => "crawl_stop",
                CrawlAction::Restart => "crawl_restart",
                CrawlAction::Status => "crawl_status",
            };
            cli_call(&cli.socket, msg).await?;
        }
        _ => {}
    }
    Ok(())
}

async fn cli_call(socket: &PathBuf, cmd: &str) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;
    let mut stream = UnixStream::connect(socket).await?;
    stream.write_all(format!("{}\n", cmd).as_bytes()).await?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).await?;
    print!("{}", buf);
    Ok(())
}
