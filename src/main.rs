#![allow(unused_variables, dead_code)]
use anyhow::Result;
use clap::Parser;
use log::{debug, error};
use std::sync::Arc;
use tokio::{
    net::{TcpListener, TcpStream},
    select,
    signal::unix::{SignalKind, signal},
    time::Duration,
};

mod docker_mgr;
use docker_mgr::DockerManager;

mod iotracker;
use iotracker::AsyncRWTracker;

/// Simple proxy program to manage containers
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
struct Args {
    /// Port to listen on
    #[arg(short, long)]
    port: u16,

    /// Server address
    #[arg(long)]
    host: String,

    /// Container group name
    #[arg(short, long)]
    group: String,

    /// Container idle time (seconds)
    #[arg(long, default_value_t = 300)]
    idle: u64,

    /// Disable docker health check
    #[arg(long, default_value_t = false)]
    no_health: bool,
}

async fn tcp_listener(docker: Arc<DockerManager>, port: u16, host: String) -> Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;

    loop {
        let (mut inbound, _) = listener.accept().await?;

        let docker = docker.clone();
        let host = host.clone();
        tokio::spawn(async move {
            loop {
                if docker.wait_healthy().await.is_err() {
                    error!("Docker image never became healthy; exiting");
                    break;
                };

                let outbound = TcpStream::connect(&host).await;
                let Ok(outbound) = outbound else {
                    error!("Connection failed, retrying in 2 seconds");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                };

                debug!("Connection made, tracking...");

                let mut outbound = AsyncRWTracker::new(docker.sender.clone(), outbound);
                if let Err(err) = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await {
                    debug!("Error: {}", err);
                }
                debug!("Connection closed");
                break;
            }
        });
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .init();

    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or("panic occurred".to_string());
        error!("{}", msg);

        std::process::exit(128 + 6); // SIGABRT
    }));

    tokio::spawn(async move {
        let mut sigterm = signal(SignalKind::terminate()).unwrap();
        let mut sigint = signal(SignalKind::interrupt()).unwrap();
        select! {
            _ = sigterm.recv() => std::process::exit(128+15),
            _ = sigint.recv() => std::process::exit(128+2),
        };
    });

    let docker = DockerManager::new(args.clone()).unwrap();
    tcp_listener(Arc::new(docker), args.port, args.host.clone())
        .await
        .expect("Failed to start listener");
}
