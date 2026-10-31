//! JSON parsing helpers module
//!
//! This module provides utilities for parsing JSON configuration values.

use serde_json::{Map, Value};

/// Parse integer from JSON value
pub fn int_value(v: &Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    if v.is_number() {
        if let Some(f) = v.as_f64() {
            if f >= 0.0 && f.fract() == 0.0 && f <= u64::MAX as f64 {
                return Some(f as u64);
            }
        }
    }
    None
}

/// Parse number field with validation
pub fn number_field(
    obj: &Map<String, Value>,
    key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, String> {
    match obj.get(key) {
        None => Ok(default),
        Some(v) => {
            let n = match int_value(v) {
                Some(n) => n,
                None => return Err(format!("\"{}\" must be an integer", key)),
            };
            if n < min || n > max {
                return Err(format!("\"{}\" must be between {} and {}", key, min, max));
            }
            Ok(n)
        }
    }
}
