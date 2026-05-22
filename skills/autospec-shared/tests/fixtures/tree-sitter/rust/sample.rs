// sample.rs — Rust fixture for autospec-docs walker unit tests.

use std::collections::HashMap;
use std::fs;

pub const MAX_CONNECTIONS: u32 = 100;

pub struct Config {
    pub host: String,
    pub port: u16,
}

pub fn parse_config(path: &str) -> Result<Config, String> {
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    // Simplified: real impl would use serde_json
    let _ = content;
    Ok(Config {
        host: "localhost".to_string(),
        port: 8080,
    })
}

pub fn format_address(config: &Config) -> String {
    format!("{}:{}", config.host, config.port)
}

pub trait Handler {
    fn handle(&self, request: &str) -> String;
}

fn main() {
    let cfg = parse_config("config.json").unwrap_or(Config {
        host: "localhost".to_string(),
        port: 8080,
    });
    println!("{}", format_address(&cfg));
}
