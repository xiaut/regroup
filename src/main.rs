// regroup — find machines by their SSH host key fingerprint and update ~/.ssh/config.
//
// NEW ARCHITECTURE: Configuration is stored in a separate git repository
// (regroup-config or regroup-conf) where each device has its own JSON file in the
// devices/ directory. Devices auto-fill their own information if missing.
//
// One binary, bidirectional: every run announces this machine ("name ip") over
// UDP broadcast AND listens for the other machines' announcements during the
// same window. Candidates are resolved by fingerprint
// (ssh-keyscan | ssh-keygen -lf -), so random hotspot subnets are no problem.
// A machine listed in the config that turns out to be THIS machine is detected
// as local and never written into its own ~/.ssh/config.
//
// Build (Termux):  pkg install rust openssh && cargo build --release
// Build (Ubuntu): sudo apt install cargo && cargo build --release
//
// Run:            ./regroup                (announce + listen, update ~/.ssh/config)
//                 ./regroup test           (dry run: nothing is modified)
//                 ./regroup -v             (verbose)
//                 ./regroup -c REPO        (config repository path or URL)
//                 ./regroup -r             (refresh config from git repo)
//                 ./regroup 192.168.43     (also scan that /24 once)
//                 ./regroup selftest       (built-in checks)
//                 ./regroup -h             (help)
//
// Config: A separate git repo (default: ./regroup-config) with:
//   devices/<name>.json - each device's configuration
//   Each file can contain: name, fingerprint, user, port
//   If a device's file is missing, it auto-creates it with self-detected info
//
// Exit codes: 0 = all machines found; 1 = at least one missing;
//             2 = configuration or environment error.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

// ============================================================
// Data types
// ============================================================

struct Machine {
    name: String,
    fingerprint: String,
    user: String,
    port: u16,
    port_explicit: bool,
}

struct Conf {
    udp_port: u16,
    wait_secs: u64,
    batch_size: usize,
    subnets: Vec<String>,
    machines: Vec<Machine>,
    config_dir: PathBuf,
}

// ============================================================
// Configuration constants
// ============================================================

const DEFAULT_CONFIG_REPO: &str = "regroup-config";
const DEVICES_DIR: &str = "devices";
const GLOBAL_CONFIG_FILE: &str = "settings.json";

// ============================================================
// Error handling
// ============================================================

fn die(msg: &str) -> ! {
    eprintln!("ERROR: {}", msg);
    std::process::exit(2);
}

fn warn(msg: &str) {
    eprintln!("WARNING: {}", msg);
}

// ============================================================
// Git operations
// ============================================================

/// Check if a directory is a git repository
fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// Clone a git repository
fn git_clone(repo_url: &str, target_dir: &Path) -> io::Result<()> {
    if target_dir.exists() {
        return Ok(()); // Already exists
    }
    
    // Create parent directory if needed
    if let Some(parent) = target_dir.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    
    let status = Command::new("git")
        .arg("clone")
        .arg(repo_url)
        .arg(target_dir)
        .status()?;
    
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("git clone failed with exit code: {}", status),
        ));
    }
    
    Ok(())
}

/// Pull updates from git repository
fn git_pull(repo_path: &Path) -> io::Result<()> {
    let status = Command::new("git")
        .current_dir(repo_path)
        .arg("pull")
        .status()?;
    
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("git pull failed with exit code: {}", status),
        ));
    }
    
    Ok(())
}

/// Initialize a new git repository
fn git_init(repo_path: &Path) -> io::Result<()> {
    if repo_path.exists() {
        return Ok(());
    }
    
    fs::create_dir_all(repo_path)?;
    
    let status = Command::new("git")
        .current_dir(repo_path)
        .arg("init")
        .status()?;
    
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "git init failed",
        ));
    }
    
    // Configure git to allow commits without user info
    let _ = Command::new("git")
        .current_dir(repo_path)
        .arg("config")
        .arg("user.email")
        .arg("regroup@local")
        .status();
    
    let _ = Command::new("git")
        .current_dir(repo_path)
        .arg("config")
        .arg("user.name")
        .arg("regroup")
        .status();
    
    Ok(())
}

/// Get the fingerprint of the local machine
fn get_local_fingerprint(port: u16) -> Option<String> {
    if let Some(ip) = my_ip() {
        let fps = fingerprints(&ip, port, false);
        if !fps.is_empty() {
            return Some(fps[0].clone());
        }
    }
    None
}

// ============================================================
// Device configuration
// ============================================================

/// Load a single device configuration from a JSON file
fn load_device_file(path: &Path) -> Result<Machine, String> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return Err(format!("cannot read {}: {}", path.display(), e)),
    };
    
    let root: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return Err(format!("invalid JSON in {}: {}", path.display(), e)),
    };
    
    let obj = match root.as_object() {
        Some(o) => o,
        None => return Err(format!("{}: top-level value must be a JSON object", path.display())),
    };
    
    let name = match obj.get("name") {
        Some(v) => match v.as_str() {
            Some(s) => s.trim().to_string(),
            None => return Err(format!("{}: \"name\" must be a string", path.display())),
        },
        None => {
            // Try to get name from filename
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
            if let Some(name) = filename.strip_suffix(".json") {
                name.to_string()
            } else {
                filename.to_string()
            }
        }
    };
    
    let fingerprint = match obj.get("fingerprint") {
        Some(v) => match v.as_str() {
            Some(s) => s.trim().to_string(),
            None => return Err(format!("{}: \"fingerprint\" must be a string", path.display())),
        },
        None => String::new(),
    };
    
    let user = match obj.get("user") {
        Some(v) => match v.as_str() {
            Some(s) => s.trim().to_string(),
            None => return Err(format!("{}: \"user\" must be a string", path.display())),
        },
        None => String::new(),
    };
    
    let (port, port_explicit) = match obj.get("port") {
        None => (22u16, false),
        Some(v) => {
            let n = match int_value(v) {
                Some(n) => n,
                None => return Err(format!("{}: \"port\" must be an integer", path.display())),
            };
            if n < 1 || n > 65535 {
                return Err(format!(
                    "{}: \"port\" must be between 1 and 65535",
                    path.display()
                ));
            }
            (n as u16, true)
        }
    };
    
    Ok(Machine {
        name,
        fingerprint,
        user,
        port,
        port_explicit,
    })
}

/// Save a device configuration to a JSON file
fn save_device_file(path: &Path, machine: &Machine) -> io::Result<()> {
    let mut obj = serde_json::Map::new();
    obj.insert("name".to_string(), serde_json::Value::String(machine.name.clone()));
    obj.insert("fingerprint".to_string(), serde_json::Value::String(machine.fingerprint.clone()));
    
    if !machine.user.is_empty() {
        obj.insert("user".to_string(), serde_json::Value::String(machine.user.clone()));
    }
    
    if machine.port_explicit {
        obj.insert("port".to_string(), serde_json::Value::Number(machine.port.into()));
    }
    
    let value = serde_json::Value::Object(obj);
    fs::write(path, serde_json::to_string_pretty(&value)?)?;
    Ok(())
}

/// Auto-create device configuration with self-detected information
fn create_device_config(config_dir: &Path, device_name: &str) -> Result<Machine, String> {
    let hostname = host_name();
    let fingerprint = get_local_fingerprint(22).unwrap_or_default();
    
    // Use hostname as name if device_name is empty
    let name = if device_name.is_empty() {
        hostname
    } else {
        device_name.to_string()
    };
    
    // Determine the current user
    let current_user = env::var("USER").unwrap_or_else(|_| env::var("LOGNAME").unwrap_or_default());
    
    let machine = Machine {
        name: name.clone(),
        fingerprint: if fingerprint.is_empty() { 
            "SHA256:PUT-YOUR-FINGERPRINT-HERE".to_string()
        } else {
            fingerprint
        },
        user: current_user,
        port: 22,
        port_explicit: false,
    };
    
    // Save to device file
    let devices_dir = config_dir.join(DEVICES_DIR);
    if !devices_dir.exists() {
        if let Err(e) = fs::create_dir_all(&devices_dir) {
            return Err(format!("cannot create devices directory: {}", e));
        }
    }
    
    let device_path = devices_dir.join(format!("{}.json", name));
    if let Err(e) = save_device_file(&device_path, &machine) {
        return Err(format!("cannot save device config: {}", e));
    }
    
    Ok(machine)
}

// ============================================================
// Global configuration
// ============================================================

/// Load global settings from settings.json
fn load_global_settings(config_dir: &Path) -> (u16, u64, usize, Vec<String>) {
    let settings_path = config_dir.join(GLOBAL_CONFIG_FILE);
    
    let defaults = (4242u16, 10u64, 64usize, Vec::<String>::new());
    
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
            match net_valid(&net_prefix(s)) {
                Some(n) => subnets.push(n),
                None => continue,
            }
        }
    }
    
    (udp_port, wait_secs, batch_size, subnets)
}

/// Save default global settings
fn save_default_global_settings(config_dir: &Path) -> io::Result<()> {
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

// ============================================================
// Configuration loading
// ============================================================

fn load_config(config_path: &Path) -> Conf {
    let config_dir = if config_path.exists() && config_path.is_dir() {
        config_path.to_path_buf()
    } else if config_path.exists() {
        // If it's a file, use its parent directory
        config_path.parent().unwrap_or(config_path).to_path_buf()
    } else {
        // Try to use the path as a directory, or create it
        if let Some(parent) = config_path.parent() {
            if !parent.exists() {
                if let Err(e) = fs::create_dir_all(parent) {
                    die(&format!("cannot create config directory {}: {}", parent.display(), e));
                }
            }
        }
        config_path.to_path_buf()
    };
    
    // Check if config_dir exists, if not try to clone from default
    if !config_dir.exists() {
        // Try to clone from default repository URL
        let clone_url = format!("file://{}", DEFAULT_CONFIG_REPO);
        if git_clone(&clone_url, &config_dir).is_err() {
            // Try as a local path
            let local_path = Path::new(DEFAULT_CONFIG_REPO);
            if local_path.exists() && local_path.is_dir() {
                if let Err(e) = fs::create_dir_all(&config_dir) {
                    die(&format!("cannot create config directory: {}", e));
                }
                // Copy from local default
                warn(&format!("using local config from {}", DEFAULT_CONFIG_REPO));
            } else {
                // Initialize a new git repository
                if let Err(e) = git_init(&config_dir) {
                    die(&format!("cannot initialize git repository: {}", e));
                }
                
                // Create default settings
                if let Err(e) = save_default_global_settings(&config_dir) {
                    die(&format!("cannot create default settings: {}", e));
                }
                
                // Create devices directory
                let devices_dir = config_dir.join(DEVICES_DIR);
                if let Err(e) = fs::create_dir_all(&devices_dir) {
                    die(&format!("cannot create devices directory: {}", e));
                }
            }
        }
    }
    
    // Load global settings
    let (udp_port, wait_secs, batch_size, subnets) = load_global_settings(&config_dir);
    
    // Load all device configurations
    let mut machines: Vec<Machine> = Vec::new();
    let devices_dir = config_dir.join(DEVICES_DIR);
    
    if devices_dir.exists() {
        if let Ok(entries) = fs::read_dir(&devices_dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_file() && path.extension().map_or(false, |ext| ext == "json") {
                        match load_device_file(&path) {
                            Ok(machine) => machines.push(machine),
                            Err(e) => warn(&format!("skipping {}: {}", path.display(), e)),
                        }
                    }
                }
            }
        }
    }
    
    // Check if current machine's config exists, if not create it
    let local_hostname = host_name();
    
    let current_machine_exists = machines.iter().any(|m| {
        m.name == local_hostname || 
        (m.fingerprint.starts_with("SHA256:") && m.fingerprint == get_local_fingerprint(m.port).unwrap_or_default())
    });
    
    if !current_machine_exists {
        // Auto-create this device's configuration
        if let Ok(new_machine) = create_device_config(&config_dir, &local_hostname) {
            machines.push(new_machine);
            
            // Commit the new device config
            if is_git_repo(&config_dir) {
                let message = format!("Add device: {}", local_hostname);
                let _ = git_add_and_commit(&config_dir, &message);
            }
        }
    }
    
    // Sort machines by name for consistent ordering
    machines.sort_by(|a, b| a.name.cmp(&b.name));
    
    if machines.is_empty() {
        die(&format!(
            "no devices configured in {} — first device will be auto-created on first run",
            config_dir.display()
        ));
    }
    
    Conf {
        udp_port,
        wait_secs,
        batch_size,
        subnets,
        machines,
        config_dir,
    }
}

/// Git add and commit helper
fn git_add_and_commit(repo_path: &Path, message: &str) -> io::Result<()> {
    if !is_git_repo(repo_path) {
        return Ok(());
    }
    
    let status = Command::new("git")
        .current_dir(repo_path)
        .arg("add")
        .arg(".")
        .status()?;
    
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "git add failed",
        ));
    }
    
    let status = Command::new("git")
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg(message)
        .status()?;
    
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "git commit failed",
        ));
    }
    
    Ok(())
}

/// Pull latest changes from config repository
fn refresh_config(config_path: &Path) -> io::Result<()> {
    if !config_path.exists() || !is_git_repo(config_path) {
        return Ok(());
    }
    
    git_pull(config_path)?;
    Ok(())
}

// ============================================================
// Legacy configuration support (for backward compatibility)
// ============================================================

fn config_template() -> String {
    r#"{
    "udp_port": 4242,
    "wait_secs": 10,
    "batch_size": 64,
    "subnets": [],
    "machines": [
        {
            "name": "ordi",
            "fingerprint": "SHA256:PUT-YOUR-FINGERPRINT-HERE",
            "user": "matthias",
            "port": 22
        }
    ]
}
"#
    .to_string()
}

// Integer value from a JSON number: accepts 42 and 42.0, rejects 42.5.
fn int_value(v: &Value) -> Option<u64> {
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

fn number_field(
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

fn parse_legacy_config(text: &str) -> Result<Conf, String> {
    let root: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return Err(format!("invalid JSON: {}", e)),
    };
    let obj = match root.as_object() {
        Some(o) => o,
        None => return Err("the top-level value must be a JSON object".to_string()),
    };

    let udp_port = number_field(obj, "udp_port", 4242, 1, 65535)? as u16;
    let wait_secs = number_field(obj, "wait_secs", 10, 1, 3600)?;
    let batch_size = number_field(obj, "batch_size", 64, 1, 1024)? as usize;

    let mut subnets: Vec<String> = Vec::new();
    if let Some(v) = obj.get("subnets") {
        let arr = match v.as_array() {
            Some(a) => a,
            None => return Err("\"subnets\" must be an array of strings".to_string()),
        };
        for s in arr {
            let s = match s.as_str() {
                Some(s) => s.trim(),
                None => return Err("\"subnets\" must be an array of strings".to_string()),
            };
            if s.is_empty() {
                continue;
            }
            match net_valid(&net_prefix(s)) {
                Some(n) => subnets.push(n),
                None => {
                    return Err(format!(
                        "\"subnets\": \"{}\" is not a valid /24 prefix (e.g. \"192.168.43\")",
                        s
                    ));
                }
            }
        }
    }

    let machines_v = match obj.get("machines") {
        Some(v) => v,
        None => return Err("\"machines\" is missing (an array is required)".to_string()),
    };
    let arr = match machines_v.as_array() {
        Some(a) => a,
        None => return Err("\"machines\" must be an array".to_string()),
    };
    if arr.is_empty() {
        return Err("\"machines\" is empty — configure at least one machine".to_string());
    }

    let mut machines = Vec::new();
    for (i, mv) in arr.iter().enumerate() {
        let m = match mv.as_object() {
            Some(m) => m,
            None => return Err(format!("machines[{}] must be a JSON object", i)),
        };
        let name = required_str(m, "name", i)?;
        let fingerprint = required_str(m, "fingerprint", i)?;
        let user = match m.get("user") {
            None => String::new(),
            Some(v) => match v.as_str() {
                Some(s) => s.trim().to_string(),
                None => return Err(format!("machines[{}]: \"user\" must be a string", i)),
            },
        };
        let (port, port_explicit) = match m.get("port") {
            None => (22u16, false),
            Some(v) => {
                let n = match int_value(v) {
                    Some(n) => n,
                    None => return Err(format!("machines[{}]: \"port\" must be an integer", i)),
                };
                if n < 1 || n > 65535 {
                    return Err(format!(
                        "machines[{}]: \"port\" must be between 1 and 65535",
                        i
                    ));
                }
                (n as u16, true)
            }
        };
        if !(fingerprint.starts_with("SHA256:") || fingerprint.starts_with("MD5:")) {
            warn(&format!(
                "machines[{}]: fingerprint has an unusual format (\"SHA256:...\" expected)",
                i
            ));
        }
        machines.push(Machine {
            name,
            fingerprint,
            user,
            port,
            port_explicit,
        });
    }

    let mut names: Vec<&str> = machines.iter().map(|m| m.name.as_str()).collect();
    names.sort_unstable();
    for w in names.windows(2) {
        if w[0] == w[1] {
            return Err(format!("duplicate machine name: \"{}\"", w[0]));
        }
    }

    Ok(Conf {
        udp_port,
        wait_secs,
        batch_size,
        subnets,
        machines,
        config_dir: PathBuf::from("legacy"),
    })
}

fn required_str(m: &Map<String, Value>, key: &str, i: usize) -> Result<String, String> {
    match m.get(key) {
        Some(v) => {
            let s = match v.as_str() {
                Some(s) => s,
                None => return Err(format!("machines[{}]: \"{}\" must be a string", i, key)),
            }
            .trim();
            if s.is_empty() {
                return Err(format!("machines[{}]: \"{}\" must not be empty", i, key));
            }
            Ok(s.to_string())
        }
        None => Err(format!("machines[{}]: \"{}\" is required", i, key)),
    }
}

// ============================================================
// Subnet helpers
// ============================================================

// Keeps at most the first three dot-separated parts ("192.168.43.0/24" -> "192.168.43").
fn net_prefix(s: &str) -> String {
    let parts: Vec<&str> = s.split('.').collect();
    let n = parts.len().min(3);
    parts[..n].join(".")
}

// Normalizes a /24 prefix; Some("192.168.43") if valid, None otherwise.
fn net_valid(s: &str) -> Option<String> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut octets: Vec<u32> = Vec::new();
    for p in parts {
        if p.is_empty() || p.len() > 3 {
            return None;
        }
        if !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let v: u32 = match p.parse() {
            Ok(v) => v,
            Err(_) => return None,
        };
        if v > 255 {
            return None;
        }
        octets.push(v);
    }
    let out: Vec<String> = octets.iter().map(|o| o.to_string()).collect();
    Some(out.join("."))
}

// ============================================================
// Local identity
// ============================================================

fn host_name() -> String {
    if let Ok(o) = Command::new("hostname").output() {
        if o.status.success() {
            let n = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !n.is_empty() {
                return n;
            }
        }
    }
    "machine".to_string()
}

fn my_ip() -> Option<String> {
    // "Connecting" a UDP socket sends no packet but selects the default
    // interface and reveals its source address.
    if let Ok(s) = UdpSocket::bind("0.0.0.0:0") {
        if s.connect("1.1.1.1:80").is_ok() {
            if let Ok(a) = s.local_addr() {
                let ip = a.ip().to_string();
                if ip.parse::<Ipv4Addr>().is_ok() {
                    return Some(ip);
                }
            }
        }
    }
    // Fallback: ask the kernel via "ip route get".
    if let Ok(o) = Command::new("ip")
        .args(["route", "get", "1.1.1.1"])
        .output()
    {
        if o.status.success() {
            let words: Vec<String> = String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .map(ToString::to_string)
                .collect();
            for (i, w) in words.iter().enumerate() {
                if *w == "src" && i + 1 < words.len() {
                    return Some(words[i + 1].to_string());
                }
            }
        }
    }
    None
}

// ============================================================
// Beacons (UDP)
// ============================================================

// Broadcasts "name ip" every 2 s until the deadline. Runs in its own thread,
// in parallel with listen_beacons, for the same window.
fn announce_beacon(port: u16, secs: u64, name: &str, ip: &str) {
    let target = format!("255.255.255.255:{}", port);
    let message = format!("{} {}", name, ip);
    let sock = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("WARNING: announcing disabled: {}", e);
            return;
        }
    };
    let _ = sock.set_broadcast(true);
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        let _ = sock.send_to(message.as_bytes(), &target);
        thread::sleep(Duration::from_secs(2));
    }
}

// Listens for "name ip" beacons during the window. Returns (name, ip) pairs,
// deduplicated by IP.
fn listen_beacons(port: u16, secs: u64) -> Vec<(String, String)> {
    let mut beacons: Vec<(String, String)> = Vec::new();
    let sock = match UdpSocket::bind(("0.0.0.0", port)) {
        Ok(s) => s,
        Err(e) => {
            warn(&format!(
                "cannot listen on UDP port {} ({}) — beacons ignored",
                port, e
            ));
            return beacons;
        }
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                let words: Vec<&str> = text.split_whitespace().collect();
                for w in &words {
                    if let Ok(ip) = w.parse::<Ipv4Addr>() {
                        let s = ip.to_string();
                        if !beacons.iter().any(|(_, i)| i == &s) {
                            let name = words
                                .iter()
                                .find(|x| x.parse::<Ipv4Addr>().is_err())
                                .map(|x| x.to_string())
                                .unwrap_or_default();
                            beacons.push((name, s));
                        }
                        break;
                    }
                }
            }
            Err(_) => {} // read timeout: loop until the deadline
        }
    }
    beacons
}

// ============================================================
// Fallback TCP scan
// ============================================================

// Scans a /24 on one port, `batch_size` connections in parallel.
fn scan_subnet(net: &str, port: u16, batch_size: usize) -> Vec<(String, u16)> {
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

// ============================================================
// Fingerprints (ssh-keyscan | ssh-keygen)
// ============================================================

// Collects the fingerprints of every host key offered by ip:port, using
// OpenSSH itself: ssh-keyscan fetches the keys, ssh-keygen -lf - prints
// "bits SHA256:... comment (type)" lines.
fn fingerprints(ip: &str, port: u16, md5: bool) -> Vec<String> {
    let scan = Command::new("ssh-keyscan")
        .arg("-T")
        .arg("3")
        .arg("-p")
        .arg(port.to_string())
        .arg(ip)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let raw = match scan {
        Ok(o) if !o.stdout.is_empty() => o.stdout,
        _ => return Vec::new(),
    };
    let mut cmd = Command::new("ssh-keygen");
    cmd.arg("-lf")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if md5 {
        cmd.arg("-E").arg("md5");
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&raw);
    } // stdin dropped here -> EOF for ssh-keygen
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let wanted = if md5 { "MD5:" } else { "SHA256:" };
    let mut fps: Vec<String> = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 2 && fields[1].starts_with(wanted) {
            let f = fields[1].to_string();
            if !fps.contains(&f) {
                fps.push(f);
            }
        }
    }
    fps
}

// ============================================================
// ~/.ssh/config
// ============================================================

// Replaces ONLY the "Host <name>" block; all other blocks are preserved
// (e.g. github.com). Idempotent: running it twice yields the same file.
fn update_block(
    path: &Path,
    name: &str,
    ip: &str,
    user: &str,
    port: u16,
    port_explicit: bool,
) -> io::Result<()> {
    let old = fs::read_to_string(path).unwrap_or_default();
    let mut kept = String::new();
    let mut in_block = false;
    for line in old.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if !words.is_empty() && words[0] == "Host" {
            in_block = false;
            if words[1..].iter().any(|w| *w == name) {
                in_block = true;
                continue;
            }
        }
        if in_block {
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    let body = kept.trim_end();
    let mut out = String::new();
    if !body.is_empty() {
        out.push_str(body);
        out.push_str("\n\n");
    }
    out.push_str(&format!("Host {}\n    HostName {}\n", name, ip));
    if !user.is_empty() {
        out.push_str(&format!("    User {}\n", user));
    }
    if port_explicit {
        out.push_str(&format!("    Port {}\n", port));
    }
    fs::write(path, out)
}

// ============================================================
// Selftest
// ============================================================

fn run_selftest() -> ! {
    let mut ok = true;

    // 1. external tools
    let mut tools_ok = true;
    for tool in ["ssh-keyscan", "ssh-keygen"] {
        if Command::new(tool).output().is_err() {
            tools_ok = false;
        }
    }
    // Check for git
    if Command::new("git").output().is_err() {
        tools_ok = false;
    }
    
    println!(
        "ssh-keyscan/ssh-keygen/git present — {}",
        if tools_ok { "OK" } else { "FAIL" }
    );
    ok &= tools_ok;

    // 2. config template parses + validates
    let parsed = parse_legacy_config(&config_template());
    let conf_ok = parsed.is_ok();
    println!(
        "legacy config template parses + validates — {}",
        if conf_ok { "OK" } else { "FAIL" }
    );
    ok &= conf_ok;
    if let Ok(c) = parsed {
        let v = c.udp_port == 4242
            && c.wait_secs == 10
            && c.batch_size == 64
            && c.subnets.is_empty()
            && c.machines.len() == 1
            && c.machines[0].name == "ordi"
            && c.machines[0].port == 22
            && c.machines[0].port_explicit;
        println!("legacy config template fields — {}", if v { "OK" } else { "FAIL" });
        ok &= v;
    }

    // 3. subnet helpers
    let net_ok = net_valid(&net_prefix("192.168.43.0/24")) == Some("192.168.43".to_string())
        && net_valid(&net_prefix("192.168.43")) == Some("192.168.43".to_string())
        && net_valid("192.168.43") == Some("192.168.43".to_string())
        && net_valid("192.168.256") == None
        && net_valid("192.168") == None
        && net_valid("abc.def.ghi") == None;
    println!("subnet helpers — {}", if net_ok { "OK" } else { "FAIL" });
    ok &= net_ok;

    println!("selftest: {}", if ok { "OK" } else { "FAIL" });
    std::process::exit(if ok { 0 } else { 1 });
}

// ============================================================
// Program entry
// ============================================================

fn usage() {
    println!(
        "regroup — find machines by their SSH host key fingerprint and update
~/.ssh/config. Every run announces this machine over UDP broadcast while
listening for the other machines' announcements during the same window.

NEW: Configuration is now stored in a separate git repository where each
device has its own file. Devices auto-fill their information if missing.

Usage:
  ./regroup                announce + listen, then update ~/.ssh/config
  ./regroup test           dry run: nothing is modified
  ./regroup -v             verbose
  ./regroup -c REPO        use REPO as config repository (default: ./regroup-config)
  ./regroup -r             refresh config from git repo before running
  ./regroup 192.168.43     also scan that /24 once (if broadcast is filtered)
  ./regroup selftest       built-in checks (tools, config, config rewrite)
  ./regroup -h             this help

Config repository structure:
  REPO/
    settings.json           (optional: udp_port, wait_secs, batch_size, subnets)
    devices/
      <hostname>.json       (each device's config: name, fingerprint, user, port)

On first run, a new git repository is created with a devices/ directory.
Each device auto-creates its own config file with detected information.
Sync the repository across devices on a controlled network.

Exit codes:
  0  all machines found
  1  at least one machine missing
  2  configuration or environment error"
    );
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    let mut verbose = false;
    let mut dry_run = false;
    let mut help = false;
    let mut selftest = false;
    let mut net_arg: Option<String> = None;
    let mut conf_path = PathBuf::from(DEFAULT_CONFIG_REPO);
    let mut refresh = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "test" {
            dry_run = true;
        } else if a == "-v" || a == "--verbose" {
            verbose = true;
        } else if a == "-h" || a == "--help" {
            help = true;
        } else if a == "selftest" {
            selftest = true;
        } else if a == "-c" || a == "--config" {
            i += 1;
            if i >= args.len() {
                die("option -c/--config requires a path or URL");
            }
            conf_path = PathBuf::from(&args[i]);
        } else if a == "-r" || a == "--refresh" {
            refresh = true;
        } else if a.contains('.') {
            net_arg = Some(a.clone());
        } else {
            eprintln!("ERROR: unknown option: {}", a);
            usage();
            std::process::exit(2);
        }
        i += 1;
    }
    if help {
        usage();
        return;
    }
    if selftest {
        run_selftest();
    }

    // --- external tools ---
    for tool in ["ssh-keyscan", "ssh-keygen"] {
        if Command::new(tool).output().is_err() {
            die(&format!(
                "command \"{}\" not found — Termux: pkg install openssh; Ubuntu: sudo apt install openssh-client",
                tool
            ));
        }
    }
    
    // Check for git (optional, but needed for config management)
    if refresh || !conf_path.exists() {
        if Command::new("git").output().is_err() {
            warn("git not found — config repository features disabled");
        }
    }

    // --- configuration ---
    if refresh {
        if let Err(e) = refresh_config(&conf_path) {
            warn(&format!("could not refresh config: {}", e));
        }
    }
    
    let conf = load_config(&conf_path);

    // --- ~/.ssh preparation (only when we may write) ---
    let home = match env::var("HOME") {
        Ok(h) => h,
        Err(_) => die("HOME variable not set"),
    };
    let ssh_dir = PathBuf::from(&home).join(".ssh");
    let ssh_config = ssh_dir.join("config");
    if !dry_run {
        if !ssh_dir.exists() && fs::create_dir(&ssh_dir).is_err() {
            die("cannot create ~/.ssh");
        }
        let _ = fs::set_permissions(&ssh_dir, fs::Permissions::from_mode(0o700));
        if !ssh_config.exists() && fs::write(&ssh_config, "").is_err() {
            die("cannot create ~/.ssh/config");
        }
        let _ = fs::set_permissions(&ssh_config, fs::Permissions::from_mode(0o600));
        if fs::OpenOptions::new()
            .append(true)
            .open(&ssh_config)
            .is_err()
        {
            die("~/.ssh/config is not writable");
        }
    }

    // --- local identity ---
    let local_name = host_name();
    let local_ip = my_ip();
    if verbose {
        println!(
            "  . this machine: {} / {}",
            local_name,
            local_ip.as_deref().unwrap_or("unknown IP")
        );
        println!("  . config dir: {}", conf.config_dir.display());
    }

    // --- announce (background thread) + listen, same window ---
    println!(
        "== Announcing + listening for beacons ({} s, UDP port {}) ==",
        conf.wait_secs, conf.udp_port
    );
    let announcer = match local_ip.clone() {
        Some(ip) => {
            let port = conf.udp_port;
            let secs = conf.wait_secs;
            let name = local_name.clone();
            Some(thread::spawn(move || {
                announce_beacon(port, secs, &name, &ip)
            }))
        }
        None => {
            warn("local IP unknown — announcing disabled (listening continues)");
            None
        }
    };
    let beacons = listen_beacons(conf.udp_port, conf.wait_secs);
    if let Some(h) = announcer {
        let _ = h.join();
    }
    if verbose {
        println!("  . {} beacon(s) received", beacons.len());
        for (n, ip) in &beacons {
            println!(
                "  . beacon: {} ({})",
                if n.is_empty() { "?" } else { n.as_str() },
                ip
            );
        }
    }

    // --- candidates: beacons x ports ---
    let mut ports: Vec<u16> = conf.machines.iter().map(|m| m.port).collect();
    ports.sort_unstable();
    ports.dedup();

    let mut candidates: Vec<(String, u16)> = Vec::new();
    for (_, ip) in &beacons {
        for p in &ports {
            candidates.push((ip.clone(), *p));
        }
    }

    // --- fallback (or requested) subnet scan ---
    let mut nets: Vec<String> = Vec::new();
    match &net_arg {
        Some(a) => match net_valid(&net_prefix(a)) {
            Some(n) => nets.push(n),
            None => die(&format!(
                "argument \"{}\" is not a valid /24 prefix (e.g. 192.168.43)",
                a
            )),
        },
        None => nets.extend(conf.subnets.iter().cloned()),
    }
    let force_scan = net_arg.is_some();
    if (candidates.is_empty() || force_scan) && !nets.is_empty() {
        println!(
            "== Subnet scan ({}) ==",
            if force_scan { "requested" } else { "fallback" }
        );
        for net in &nets {
            for p in &ports {
                println!("  scanning {}.0/24 (port {}) ...", net, p);
                candidates.extend(scan_subnet(net, *p, conf.batch_size));
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    if verbose {
        println!("  . {} candidate(s) in total", candidates.len());
        for (ip, p) in &candidates {
            println!("  . candidate {}:{}", ip, p);
        }
    }

    // --- fingerprints of the candidates ---
    println!("== Fingerprinting candidates ==");
    let md5_needed = conf
        .machines
        .iter()
        .any(|m| m.fingerprint.starts_with("MD5:"));
    let mut seen: Vec<(String, u16, String)> = Vec::new();
    for (ip, port) in &candidates {
        let fps = fingerprints(ip, *port, false);
        if fps.is_empty() {
            warn(&format!(
                "port open on {}:{} but no SSH key received (not an SSH server?)",
                ip, port
            ));
            continue;
        }
        for f in fps {
            seen.push((ip.clone(), *port, f));
        }
        if md5_needed {
            for f in fingerprints(ip, *port, true) {
                seen.push((ip.clone(), *port, f));
            }
        }
    }
    if verbose {
        println!("  . {} fingerprint(s) collected", seen.len());
        for (ip, p, f) in &seen {
            println!("  . fingerprint {}:{} {}", ip, p, f);
        }
    }
    if candidates.is_empty() {
        warn(
            "no beacons received and no open ports found — run regroup on the other machine during the window, or pass a subnet: ./regroup 192.168.43",
        );
    } else if seen.is_empty() {
        warn("open ports found but no SSH server responded");
    }

    // --- fingerprints of this machine's own IP, for local detection ---
    let mut local_fps: Vec<(u16, String)> = Vec::new();
    if let Some(ip) = &local_ip {
        for p in &ports {
            for f in fingerprints(ip, *p, false) {
                local_fps.push((*p, f));
            }
            if md5_needed {
                for f in fingerprints(ip, *p, true) {
                    local_fps.push((*p, f));
                }
            }
        }
    }

    // --- machines ---
    println!("== Machines ==");
    let mut found = 0;
    let mut missing = 0;
    for m in &conf.machines {
        // (a) pre-check: this machine offers the configured key itself
        if local_fps
            .iter()
            .any(|(p, f)| *p == m.port && *f == m.fingerprint)
        {
            println!(
                "  o {}: this is the local machine (this host), config unchanged",
                m.name
            );
            found += 1;
            continue;
        }
        // resolution by fingerprint among the candidates
        let mut matched: Option<String> = None;
        for (ip, p, f) in &seen {
            if *p == m.port && *f == m.fingerprint {
                matched = Some(ip.clone());
                break;
            }
        }
        match matched {
            None => {
                let servers = seen.iter().filter(|(_, p, _)| *p == m.port).count();
                if servers > 0 {
                    println!(
                        "  x {}: {} SSH server(s) seen on port {}, none with a matching key",
                        m.name, servers, m.port
                    );
                } else {
                    println!(
                        "  x {}: not found (no SSH server on port {})",
                        m.name, m.port
                    );
                }
                missing += 1;
            }
            Some(ip) => {
                // (b) the resolved machine is this machine
                if local_ip.as_deref() == Some(ip.as_str()) {
                    println!(
                        "  o {}: this is the local machine (this host), config unchanged",
                        m.name
                    );
                    found += 1;
                } else if dry_run {
                    println!("  ~ {} -> {} (test mode, config not modified)", m.name, ip);
                    found += 1;
                } else {
                    match update_block(&ssh_config, &m.name, &ip, &m.user, m.port, m.port_explicit)
                    {
                        Ok(()) => println!("  + {} -> {}  (~/.ssh/config updated)", m.name, ip),
                        Err(e) => {
                            warn(&format!("could not write ~/.ssh/config: {}", e));
                            missing += 1;
                            continue;
                        }
                    }
                    // purge any stale known_hosts entry for this IP
                    let _ = Command::new("ssh-keygen").arg("-R").arg(&ip).output();
                    found += 1;
                }
            }
        }
    }

    println!("== {} found, {} missing ==", found, missing);
    std::process::exit(if missing == 0 { 0 } else { 1 });
}
