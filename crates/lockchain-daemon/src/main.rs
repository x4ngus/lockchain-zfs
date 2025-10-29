use anyhow::{Context, Result};
use lockchain_core::{
    config::LockchainConfig,
    logging,
    service::{LockchainService, UnlockOptions},
};
use lockchain_zfs::SystemZfsProvider;
use log::{error, info, warn};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::{
    net::TcpListener,
    select, signal,
    sync::watch,
    time::{interval, Duration, Instant},
};

mod usb;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(err) = run().await {
        error!("daemon exit: {err:?}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    logging::init("info");
    let config_path =
        std::env::var("LOCKCHAIN_CONFIG").unwrap_or_else(|_| "/etc/lockchain-zfs.toml".to_string());
    let config = Arc::new(
        LockchainConfig::load(&config_path)
            .with_context(|| format!("load config {config_path}"))?,
    );

    info!("LockChain daemon booting (config: {config_path})");

    let provider = SystemZfsProvider::from_config(&config).context("initialise zfs provider")?;
    let service = Arc::new(LockchainService::new(config.clone(), provider));

    // health status broadcast (true = ready, false = degraded)
    let (health_tx, health_rx) = watch::channel(false);

    let usb_handle = tokio::spawn(usb::watch_usb(config.clone(), health_tx.clone()));
    let unlock_handle = tokio::spawn(periodic_unlock(
        service.clone(),
        config.clone(),
        health_tx.clone(),
    ));
    let health_handle = tokio::spawn(health_server(health_rx));

    select! {
        res = usb_handle => res??,
        res = unlock_handle => res??,
        res = health_handle => res??,
        _ = signal::ctrl_c() => {
            info!("received shutdown signal");
        }
    }

    Ok(())
}

async fn periodic_unlock(
    service: Arc<LockchainService<SystemZfsProvider>>,
    config: Arc<LockchainConfig>,
    health_tx: watch::Sender<bool>,
) -> Result<()> {
    let mut ticker = interval(Duration::from_secs(30));
    let mut last_success = Instant::now();
    loop {
        ticker.tick().await;
        let dataset = config.policy.datasets.first().cloned().unwrap_or_default();
        if dataset.is_empty() {
            warn!("no datasets configured; daemon idle");
            continue;
        }

        match service.unlock_with_retry(&dataset, UnlockOptions::default()) {
            Ok(report) => {
                if report.already_unlocked {
                    info!("dataset {dataset} already unlocked");
                } else {
                    info!("unlocked {dataset} with {} nodes", report.unlocked.len());
                }
                let _ = health_tx.send(true);
                last_success = Instant::now();
            }
            Err(err) => {
                warn!("unlock attempt failed for {dataset}: {err}");
                let _ = health_tx.send(false);
                // degrade if failure lasts >5 minutes
                if last_success.elapsed() > Duration::from_secs(300) {
                    warn!(
                        "dataset {dataset} has been locked for {:?}",
                        last_success.elapsed()
                    );
                }
            }
        }
    }
}

async fn health_server(status_rx: watch::Receiver<bool>) -> Result<()> {
    let addr: SocketAddr = std::env::var("LOCKCHAIN_HEALTH_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8787".to_string())
        .parse()
        .context("parse LOCKCHAIN_HEALTH_ADDR")?;

    let listener = TcpListener::bind(addr).await?;
    info!("health endpoint listening on http://{addr}");

    loop {
        let (mut stream, peer) = listener.accept().await?;
        let healthy = *status_rx.borrow();
        let body = if healthy { "OK" } else { "DEGRADED" };
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        if let Err(err) = stream.write_all(response.as_bytes()).await {
            warn!("failed to respond to {peer}: {err}");
        }
    }
}
