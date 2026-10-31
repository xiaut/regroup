//! Global settings module
//!
//! This module handles loading and saving global configuration settings.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde_json::Value;

use super::GLOBAL_CONFIG_FILE;
use super::json_helpers::number_field;
use crate::discovery::{net_prefix, net_valid};

/// Default global settings
pub fn default_global_settings() -> (u16, u64, usize, Vec<String>) {
    (4242u16, 10u64, 64usize, Vec::<String>::new())
}

/// Load global settings from settings.json
pub fn load_global_settings(config_dir: &PathBuf) -> (u16, u64, usize, Vec<String>) {
    let settings_path = config_dir.join(GLOBAL_CONFIG_FILE);
    
    let defaults = default_global_settings();
    
    if !settings_path.exists() {
        return defaults;
    }
    
    let text = match fs::read_to_string(&settings_path) {
        Ok(t) => t,
        Err(_) => return defaults,
    };
    
    let root: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return defaults,
    };
    
    let obj = match root.as_object() {
        Some(o) => o,
        None => return defaults,
    };
    
    let udp_port = number_field(obj, "udp_port", 4242, 1, 65535).unwrap_or(4242) as u16;
    let wait_secs = number_field(obj, "wait_secs", 10, 1, 3600).unwrap_or(10);
    let batch_size = number_field(obj, "batch_size", 64, 1, 1024).unwrap_or(64) as usize;
    
    let mut subnets: Vec<String> = Vec::new();
    if let Some(v) = obj.get("subnets") {
        let arr = match v.as_array() {
            Some(a) => a,
            None => return defaults,
        };
        for s in arr {
            let s = match s.as_str() {
                Some(s) => s.trim(),
                None => continue,
            };
            if s.is_empty() {
                continue;
            }
            if let Some(n) = net_valid(&net_prefix(s)) {
                subnets.push(n);
            }
        }
    }
    
    (udp_port, wait_secs, batch_size, subnets)
}

/// Save default global settings
pub fn save_default_global_settings(config_dir: &PathBuf) -> io::Result<()> {
    let settings_path = config_dir.join(GLOBAL_CONFIG_FILE);
    let settings = serde_json::json!({
        "udp_port": 4242,
        "wait_secs": 10,
        "batch_size": 64,
        "subnets": []
    });
    
    fs::write(settings_path, serde_json::to_string_pretty(&settings)?)?;
    Ok(())
}
