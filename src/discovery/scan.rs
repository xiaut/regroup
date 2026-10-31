//! TCP subnet scan module
//!
//! This module handles fallback TCP scanning for device discovery.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::net::TcpStream;

/// Scans a /24 on one port, `batch_size` connections in parallel.
pub fn scan_subnet(net: &str, port: u16, batch_size: usize) -> Vec<(String, u16)> {
    let results = Arc::new(Mutex::new(Vec::<(String, u16)>::new()));
    let mut start: u32 = 1;
    while start <= 254 {
        let end = (start + batch_size as u32 - 1).min(254);
        let mut handles = Vec::new();
        for k in start..=end {
            let net2 = net.to_string();
            let results2 = Arc::clone(&results);
            handles.push(thread::spawn(move || {
                let addr = format!("{}.{}:{}", net2, k, port);
                if let Ok(sa) = addr.parse::<SocketAddr>() {
                    if TcpStream::connect_timeout(&sa, Duration::from_secs(1)).is_ok() {
                        results2
                            .lock()
                            .unwrap()
                            .push((format!("{}.{}", net2, k), port));
                    }
                }
            }));
        }
        for h in handles {
            let _ = h.join();
        }
        start = end + 1;
    }
    results.lock().unwrap().clone()
}
