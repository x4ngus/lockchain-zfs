use serde::{Serialize, Deserialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
pub datasets: Vec<String>,
pub token_path: PathBuf,
}

impl Default for Config {
fn default() -> Self {
Self { datasets: vec![], token_path: PathBuf::from("/run/lockchain/token.key") }
}
}
