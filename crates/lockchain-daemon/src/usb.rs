use anyhow::Result;
use lockchain_core::LockchainConfig;
use log::info;
use std::future::pending;
use std::sync::Arc;
use tokio::sync::watch;

/// Placeholder for future integration with lockchain-key-usb.
pub async fn watch_usb(
    _config: Arc<LockchainConfig>,
    health_tx: watch::Sender<bool>,
) -> Result<()> {
    // For now, just mark the daemon healthy once at startup.
    info!("usb watcher placeholder active");
    let _ = health_tx.send(true);

    pending::<()>().await;
    #[allow(unreachable_code)]
    Ok(())
}
