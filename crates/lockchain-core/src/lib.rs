pub mod config;
pub mod error;

use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct AppConfig {
pub datasets: Vec<String>,
pub usb_fingerprint: Option<String>,
}
