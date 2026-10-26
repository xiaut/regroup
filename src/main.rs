// regroup — find machines by their SSH host key fingerprint and update ~/.ssh/config.
//
// Configuration is stored in ~/.regroup-config/ git repository where each device
// has its own JSON file in the devices/ directory. Git remotes are automatically
// configured for all other devices to enable decentralized sync.
//
// Discovery and sync run in a loop until no new devices are found, handling IP
// address changes automatically. Each device owns its own config file and can only
// overwrite it. Conflicts on self-config are resolved by asking the user.
//
// Every run announces this machine ("name ip") over UDP broadcast AND listens
// for the other machines' announcements during the same window. Candidates are
// resolved by fingerprint (ssh-keyscan | ssh-keygen -lf -), so random hotspot
// subnets are no problem. A machine listed in the config that turns out to be
// THIS machine is detected as local and never written into its own ~/.ssh/config.
//
// Build (Termux):  pkg install rust openssh && cargo build --release
// Build (Ubuntu): sudo apt install cargo && cargo build --release
//
// Run:            ./regroup                (announce + listen + sync, update ~/.ssh/config)
//                 ./regroup test           (dry run: nothing is modified)
//                 ./regroup -v             (verbose)
//                 ./regroup -r             (refresh config from git remotes, then sync)
//                 ./regroup 192.168.43     (also scan that /24 once)
//                 ./regroup selftest       (built-in checks)
//                 ./regroup -h             (this help)
//
// Exit codes: 0 = all machines found; 1 = at least one missing;
//             2 = configuration or environment error; 3 = user declined conflict resolution.

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
}

// ============================================================
// Configuration constants
// ============================================================

const CONFIG_DIR: &str = ".regroup-config";
const DEVICES_DIR: &str = "devices";
const GLOBAL_CONFIG_FILE: &str = "settings.json";
const MAX_SYNC_ITERATIONS: usize = 10;

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
// User interaction
// ============================================================

/// Ask user to resolve a conflict
/// Returns: true = keep local, false = use remote, None = abort
fn ask_conflict_resolution(device_name: &str, local_content: &str, remote_content: &str) -> Option<bool> {
    use std::io::stdin;
    
    println!("\nCONFLICT: Device config for '{}' differs from remote", device_name);
    println!("Local version:");
    println!("{}", local_content);
    println!("\nRemote version:");
    println!("{}", remote_content);
    println!("\nOptions:");
    println!("  1. Keep local version");
    println!("  2. Use remote version");
    println!("  3. Abort");
    print!("\nChoice [1/2/3]: ");
    io::stdout().flush().ok();
    
    let mut input = String::new();
    if stdin().read_line(&mut input).is_ok() {
        match input.trim() {
            "1" => Some(true),
            "2" => Some(false),
            "3" => None,
            _ => Some(true), // Default to keep local
        }
    } else {
        Some(true)
    }
}

// ============================================================
// Git operations
// ============================================================

/// Check if a directory is a git repository
fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// Initialize a git repository
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
    
    // Configure git to allow pushing to the currently checked-out branch
    // This is essential for decentralized sync where each device pushes to others
    let _ = Command::new("git")
        .current_dir(repo_path)
        .arg("config")
        .arg("receive.denyCurrentBranch")
        .arg("updateInstead")
        .status();
    
    // Configure pull to use rebase by default for cleaner history
    let _ = Command::new("git")
        .current_dir(repo_path)
        .arg("config")
        .arg("pull.rebase")
        .arg("true")
        .status();
    
    Ok(())
}

/// Pull updates from all git remotes
fn git_pull_all(repo_path: &Path) -> Vec<String> {
    if !is_git_repo(repo_path) {
        return Vec::new();
    }
    
    // Check if there are any remotes configured
    let remotes = git_list_remotes(repo_path);
    if remotes.is_empty() {
        // No remotes configured - this is expected on first run
        return Vec::new();
    }
    
    // Inform user that we're about to pull from remotes (may require password)
    eprintln!("Syncing: Pulling updates from {} remote(s)...", remotes.len());
    
    let output = Command::new("git")
        .current_dir(repo_path)
        .arg("pull")
        .arg("--all")
        .arg("--rebase")
        .output();
    
    match output {
        Ok(o) if o.status.success() => Vec::new(),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            let stderr_lower = stderr.to_lowercase();
            
            // Check for authentication errors (including password prompts)
            if stderr_lower.contains("permission denied") || 
               stderr_lower.contains("authentication failed") ||
               stderr_lower.contains("no suitable response") ||
               stderr_lower.contains("connection closed by") ||
               stderr_lower.contains("impossible de lire le dépôt distant") ||
               stderr_lower.contains("could not read from remote") ||
               stderr_lower.contains("repository not found") ||
               stderr_lower.contains("dépôt non trouvé") ||
               stderr_lower.contains("password") ||
               stderr_lower.contains("mot de passe") {
                vec![String::from(
                    "SSH authentication required. To sync with remote devices:\n  1. Set up passwordless SSH authentication (recommended): ssh-copy-id user@hostname\n  2. Or ensure your SSH agent has the required keys loaded\n  3. Check that the remote device is reachable and SSH is properly configured"
                )]
            } else if stderr.contains("fatal") || stderr.contains("error") {
                // Filter out SSH usage messages (they're just noise)
                if stderr.contains("usage: ssh") {
                    vec![String::from(
                        "Git sync error. Ensure remote devices are configured correctly."
                    )]
                } else {
                    vec![stderr.trim().to_string()]
                }
            } else {
                Vec::new()
            }
        }
        Err(e) => vec![format!("{} - To enable git sync, set up SSH authentication", e)],
    }
}

/// Push to all git remotes
fn git_push_all(repo_path: &Path) -> Vec<String> {
    if !is_git_repo(repo_path) {
        return Vec::new();
    }
    
    // Check if there are any remotes configured
    let remotes = git_list_remotes(repo_path);
    if remotes.is_empty() {
        // No remotes configured - this is expected on first run
        return Vec::new();
    }
    
    // Inform user that we're about to push to remotes (may require password)
    eprintln!("Syncing: Pushing to {} remote(s)...", remotes.len());
    
    let output = Command::new("git")
        .current_dir(repo_path)
        .arg("push")
        .arg("--all")
        .arg("--force-with-lease")
        .arg("--porcelain")
        .output();
    
    match output {
        Ok(o) if o.status.success() => Vec::new(),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            let stderr_lower = stderr.to_lowercase();
            
            // Check for authentication errors
            if stderr_lower.contains("permission denied") || 
               stderr_lower.contains("authentication failed") ||
               stderr_lower.contains("no suitable response") ||
               stderr_lower.contains("connection closed by") ||
               stderr_lower.contains("impossible de lire le dépôt distant") ||
               stderr_lower.contains("could not read from remote") ||
               stderr_lower.contains("password") ||
               stderr_lower.contains("mot de passe") {
                vec![String::from(
                    "SSH authentication required. To sync with remote devices:\n  1. Set up passwordless SSH authentication (recommended): ssh-copy-id user@hostname\n  2. Or ensure your SSH agent has the required keys loaded\n  3. Check that the remote device is reachable and SSH is properly configured"
                )]
            } else if stderr_lower.contains("fatal") || stderr_lower.contains("error") {
                // Check if it's expected errors in a decentralized system
                // These are normal and shouldn't cause the program to fail
                if stderr_lower.contains("no remotes") || 
                   stderr_lower.contains("pas de destination pour pousser") ||
                   stderr_lower.contains("tip of your current branch is behind") ||
                   stderr_lower.contains("updates were rejected") ||
                   stderr_lower.contains("non-fast-forward") {
                    // These are expected in decentralized setups with one-way SSH
                    Vec::new()
                } else if stderr.contains("usage: ssh") {
                    // Filter out SSH usage messages
                    Vec::new()
                } else {
                    vec![stderr.trim().to_string()]
                }
            } else {
                Vec::new()
            }
        }
        Err(e) => vec![format!("{} - To enable git sync, set up SSH authentication", e)],
    }
}

/// Add a git remote for a machine
fn git_add_remote(repo_path: &Path, remote_name: &str, remote_url: &str) -> io::Result<()> {
    if !is_git_repo(repo_path) {
        return Ok(());
    }
    
    // Check if remote already exists
    let check = Command::new("git")
        .current_dir(repo_path)
        .arg("remote")
        .arg("get-url")
        .arg(remote_name)
        .output();
    
    match check {
        Ok(o) if o.status.success() => {
            let existing_url = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if existing_url != remote_url {
                // Update existing remote
                let status = Command::new("git")
                    .current_dir(repo_path)
                    .arg("remote")
                    .arg("set-url")
                    .arg(remote_name)
                    .arg(remote_url)
                    .status()?;
                if !status.success() {
                    return Err(io::Error::new(io::ErrorKind::Other, "failed to update remote"));
                }
            }
            Ok(())
        }
        _ => {
            // Add new remote
            let status = Command::new("git")
                .current_dir(repo_path)
                .arg("remote")
                .arg("add")
                .arg(remote_name)
                .arg(remote_url)
                .status()?;
            if !status.success() {
                return Err(io::Error::new(io::ErrorKind::Other, "failed to add remote"));
            }
            Ok(())
        }
    }
}

/// Remove a git remote
fn git_remove_remote(repo_path: &Path, remote_name: &str) -> io::Result<()> {
    if !is_git_repo(repo_path) {
        return Ok(());
    }
    
    let status = Command::new("git")
        .current_dir(repo_path)
        .arg("remote")
        .arg("remove")
        .arg(remote_name)
        .status()?;
    
    if !status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "failed to remove remote"));
    }
    
    Ok(())
}

/// Get the list of existing git remotes
fn git_list_remotes(repo_path: &Path) -> Vec<String> {
    if !is_git_repo(repo_path) {
        return Vec::new();
    }
    
    let output = Command::new("git")
        .current_dir(repo_path)
        .arg("remote")
        .output();
    
    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Refresh git remotes to match all machines except self
fn refresh_git_remotes(repo_path: &Path, machines: &[Machine], local_hostname: &str) -> Vec<String> {
    let mut errors = Vec::new();
    
    if !is_git_repo(repo_path) {
        return errors;
    }
    
    let existing_remotes = git_list_remotes(repo_path);
    
    // Remove remotes that don't correspond to any machine or are self
    for remote in &existing_remotes {
        if remote == local_hostname {
            // Never add self as a remote
            if let Err(e) = git_remove_remote(repo_path, remote) {
                errors.push(format!("failed to remove self remote {}: {}", remote, e));
            }
            continue;
        }
        
        let has_machine = machines.iter().any(|m| m.name == *remote);
        if !has_machine {
            if let Err(e) = git_remove_remote(repo_path, remote) {
                errors.push(format!("failed to remove stale remote {}: {}", remote, e));
            }
        }
    }
    
    // Add remotes for all machines (except self)
    for machine in machines {
        if machine.name == local_hostname {
            continue; // Skip self
        }
        
        // Construct SSH URL for the machine's config repo using hostname (not IP)
        let remote_url = format!("{}@{}:~/.regroup-config", machine.user, machine.name);
        
        if let Err(e) = git_add_remote(repo_path, &machine.name, &remote_url) {
            errors.push(format!("failed to add remote for {}: {}", machine.name, e));
        }
    }
    
    errors
}

/// Sync with all remotes (pull then push to all remotes)
/// Each device A can both pull from and push to all other devices
fn sync_with_remotes(repo_path: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    
    // Pull from all remotes - device A pulls from all other devices
    let pull_errors = git_pull_all(repo_path);
    errors.extend(pull_errors);
    
    // Push to all remotes - device A pushes to all other devices
    let push_errors = git_push_all(repo_path);
    errors.extend(push_errors);
    
    errors
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
// JSON helpers
// ============================================================

/// Parse integer from JSON value
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

/// Parse number field with validation
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
        None => return Err(format!("{}: \"fingerprint\" is required", path.display())),
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

/// Read device file content as string
fn read_device_file_content(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// Auto-create device configuration with self-detected information
fn create_device_config(config_dir: &Path) -> Result<Machine, String> {
    let hostname = host_name();
    let fingerprint = get_local_fingerprint(22).unwrap_or_else(|| {
        "SHA256:PUT-YOUR-FINGERPRINT-HERE".to_string()
    });
    
    // Determine the current user
    let current_user = env::var("USER").unwrap_or_else(|_| env::var("LOGNAME").unwrap_or_default());
    
    let machine = Machine {
        name: hostname.clone(),
        fingerprint,
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
    
    let device_path = devices_dir.join(format!("{}.json", hostname));
    if let Err(e) = save_device_file(&device_path, &machine) {
        return Err(format!("cannot save device config: {}", e));
    }
    
    Ok(machine)
}

// ============================================================
// Global configuration
// ============================================================

/// Default global settings
fn default_global_settings() -> (u16, u64, usize, Vec<String>) {
    (4242u16, 10u64, 64usize, Vec::<String>::new())
}

/// Load global settings from settings.json
fn load_global_settings(config_dir: &Path) -> (u16, u64, usize, Vec<String>) {
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

fn load_config() -> Conf {
    // Config directory is always ~/.regroup-config
    let home = match env::var("HOME") {
        Ok(h) => h,
        Err(_) => die("HOME variable not set"),
    };
    let config_dir = PathBuf::from(&home).join(CONFIG_DIR);
    
    // Initialize config directory if it doesn't exist
    if !config_dir.exists() {
        if let Err(e) = git_init(&config_dir) {
            die(&format!("cannot initialize config directory {}: {}", config_dir.display(), e));
        }
        
        // Create default settings
        if let Err(e) = save_default_global_settings(&config_dir) {
            die(&format!("cannot create default settings: {}", e));
        }
        
        // Commit settings.json in its own commit (separate from device files)
        let settings_path = config_dir.join(GLOBAL_CONFIG_FILE);
        if settings_path.exists() {
            let _ = git_add_and_commit_file(&config_dir, &settings_path, "Add global settings");
        }
        
        // Create devices directory
        let devices_dir = config_dir.join(DEVICES_DIR);
        if let Err(e) = fs::create_dir_all(&devices_dir) {
            die(&format!("cannot create devices directory: {}", e));
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
    let local_fingerprint = get_local_fingerprint(22);
    
    let current_machine_exists = machines.iter().any(|m| {
        m.name == local_hostname || 
        (m.fingerprint.starts_with("SHA256:") && 
         local_fingerprint.as_ref().map_or(false, |f| &m.fingerprint == f))
    });
    
    if !current_machine_exists {
        // Auto-create this device's configuration
        match create_device_config(&config_dir) {
            Ok(new_machine) => {
                machines.push(new_machine);
                
                // Commit the new device config in a separate commit
                let device_path = config_dir.join(DEVICES_DIR).join(format!("{}.json", local_hostname));
                if is_git_repo(&config_dir) && device_path.exists() {
                    let message = format!("Add device: {}", local_hostname);
                    let _ = git_add_and_commit_file(&config_dir, &device_path, &message);
                }
            }
            Err(e) => {
                warn(&format!("could not create device config: {}", e));
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
    }
}

/// Git add and commit helper (commits all changes)
/// DEPRECATED: Use git_add_and_commit_file for individual files to prevent conflicts
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

/// Git add and commit a specific file (for device files and shared files separately)
/// This ensures device-owned files are in separate commits from shared files,
/// preventing conflicts during decentralized sync.
fn git_add_and_commit_file(repo_path: &Path, file_path: &Path, message: &str) -> io::Result<()> {
    if !is_git_repo(repo_path) {
        return Ok(());
    }
    
    // Add the specific file
    let relative_path = file_path.strip_prefix(repo_path).unwrap_or(file_path);
    let status = Command::new("git")
        .current_dir(repo_path)
        .arg("add")
        .arg(relative_path)
        .status()?;
    
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "git add failed",
        ));
    }
    
    // Commit only this file
    let status = Command::new("git")
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg(message)
        .arg("--")
        .arg(relative_path)
        .status()?;
    
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "git commit failed",
        ));
    }
    
    Ok(())
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
// Discovery and Sync Loop
// ============================================================

/// Run discovery and sync in a loop until no new devices are found
fn discovery_sync_loop(
    conf: &Conf,
    ssh_config: &Path,
    local_name: &str,
    local_ip: &Option<String>,
    dry_run: bool,
    verbose: bool,
    initial_refresh: bool,
    net_arg: &Option<String>,
) -> (usize, usize) {
    let home = match env::var("HOME") {
        Ok(h) => h,
        Err(_) => die("HOME variable not set"),
    };
    let config_dir = PathBuf::from(&home).join(CONFIG_DIR);
    let devices_dir = config_dir.join(DEVICES_DIR);
    
    // Get initial list of device files
    let mut previous_device_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    if devices_dir.exists() {
        if let Ok(entries) = fs::read_dir(&devices_dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    if entry.path().is_file() && entry.path().extension().map_or(false, |ext| ext == "json") {
                        if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
                            previous_device_files.insert(name.to_string());
                        }
                    }
                }
            }
        }
    }
    
    let mut found = 0;
    let mut missing = 0;
    let mut iteration = 0;
    let mut new_devices_found = true;
    
    // Save the current state of our device file before any operations
    let local_device_path = config_dir.join(DEVICES_DIR).join(format!("{}.json", local_name));
    let our_initial_config = if local_device_path.exists() {
        read_device_file_content(&local_device_path)
    } else {
        String::new()
    };
    
    while new_devices_found && iteration < MAX_SYNC_ITERATIONS {
        iteration += 1;
        new_devices_found = false;
        
        // Update git remotes before discovery
        if !dry_run {
            let errors = refresh_git_remotes(&config_dir, &conf.machines, local_name);
            for error in &errors {
                warn(&format!("git remote refresh: {}", error));
            }
            
            // Sync with remotes before discovery
            // On first iteration, only sync if initial_refresh flag is set
            if iteration > 1 || initial_refresh {
                let sync_errors = sync_with_remotes(&config_dir);
                for error in &sync_errors {
                    // Always warn about sync errors, even in non-verbose mode
                    // The error message already includes SSH auth setup hint
                    warn(&format!("[sync] {}", error));
                    if verbose {
                        println!("  [sync] {}", error);
                    }
                }
                
                // After pulling, check if our own device config was modified
                // and if it conflicts with our initial state
                if !sync_errors.is_empty() || iteration > 1 {
                    if local_device_path.exists() {
                        // Check if the current version differs from our initial state
                        let current_content = read_device_file_content(&local_device_path);
                        
                        // Compare with initial state
                        if current_content.trim() != our_initial_config.trim() {
                            // Config was modified by the pull - ask user to resolve
                            // The current_content is what was pulled from remote
                            // our_initial_config is what we had before
                            
                            if let Some(keep_local) = ask_conflict_resolution(
                                local_name, 
                                &our_initial_config,  // Local version (before pull)
                                &current_content       // Remote version (after pull)
                            ) {
                                if keep_local {
                                    // User wants to keep local version - revert to initial
                                    if let Err(e) = fs::write(&local_device_path, &our_initial_config) {
                                        warn(&format!("failed to revert device config: {}", e));
                                    } else {
                                        // Commit the reverted device file separately
                                        let message = format!("Revert device: {} to local version", local_name);
                                        let _ = git_add_and_commit_file(&config_dir, &local_device_path, &message);
                                    }
                                } else {
                                    // User wants remote version - it's already in place from the pull
                                    // Commit it separately
                                    let message = format!("Update device: {} from remote", local_name);
                                    let _ = git_add_and_commit_file(&config_dir, &local_device_path, &message);
                                }
                            } else {
                                // User aborted
                                std::process::exit(3);
                            }
                        }
                    }
                }
            }
        }
        
        // --- announce (background thread) + listen, same window ---
        if verbose {
            println!("== Iteration {}: Announcing + listening for beacons ({} s, UDP port {}) ==",
                iteration, conf.wait_secs, conf.udp_port);
        } else if iteration == 1 {
            println!(
                "== Announcing + listening for beacons ({} s, UDP port {}) ==",
                conf.wait_secs, conf.udp_port
            );
        }
        
        let announcer = match local_ip.clone() {
            Some(ip) => {
                let port = conf.udp_port;
                let secs = conf.wait_secs;
                let name = local_name.to_string();
                Some(thread::spawn(move || {
                    announce_beacon(port, secs, &name, &ip)
                }))
            }
            None => {
                if iteration == 1 {
                    warn("local IP unknown — announcing disabled (listening continues)");
                }
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
        match net_arg {
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
            if iteration == 1 {
                println!(
                    "== Subnet scan ({}) ==",
                    if force_scan { "requested" } else { "fallback" }
                );
            }
            for net in &nets {
                if verbose {
                    println!("  scanning {}.0/24 (port {}) ...", net, ports[0]);
                }
                for p in &ports {
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
        if iteration == 1 {
            println!("== Fingerprinting candidates ==");
        }
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
        if candidates.is_empty() && iteration == 1 {
            warn(
                "no beacons received and no open ports found — run regroup on the other machine during the window, or pass a subnet: ./regroup 192.168.43",
            );
        } else if seen.is_empty() && iteration == 1 {
            warn("open ports found but no SSH server responded");
        }

        // --- fingerprints of this machine's own IP, for local detection ---
        let mut local_fps: Vec<(u16, String)> = Vec::new();
        if let Some(ip) = local_ip {
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
        if iteration == 1 {
            println!("== Machines ==");
        }
        let mut iteration_found = 0;
        let mut iteration_missing = 0;
        
        for m in &conf.machines {
            // (a) pre-check: this machine offers the configured key itself
            if local_fps
                .iter()
                .any(|(p, f)| *p == m.port && *f == m.fingerprint)
            {
                if iteration == 1 {
                    println!(
                        "  o {}: this is the local machine (this host), config unchanged",
                        m.name
                    );
                }
                iteration_found += 1;
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
                    if servers > 0 && iteration == 1 {
                        println!(
                            "  x {}: {} SSH server(s) seen on port {}, none with a matching key",
                            m.name, servers, m.port
                        );
                    } else if iteration == 1 {
                        println!(
                            "  x {}: not found (no SSH server on port {})",
                            m.name, m.port
                        );
                    }
                    iteration_missing += 1;
                }
                Some(ip) => {
                    // (b) the resolved machine is this machine
                    if local_ip.as_deref() == Some(ip.as_str()) {
                        if iteration == 1 {
                            println!(
                                "  o {}: this is the local machine (this host), config unchanged",
                                m.name
                            );
                        }
                        iteration_found += 1;
                    } else if dry_run {
                        if iteration == 1 {
                            println!("  ~ {} -> {} (test mode, config not modified)", m.name, ip);
                        }
                        iteration_found += 1;
                    } else {
                        match update_block(&ssh_config, &m.name, &ip, &m.user, m.port, m.port_explicit)
                        {
                            Ok(()) => {
                                if iteration == 1 {
                                    println!("  + {} -> {}  (~/.ssh/config updated)", m.name, ip);
                                }
                                iteration_found += 1;
                            }
                            Err(e) => {
                                warn(&format!("could not write ~/.ssh/config: {}", e));
                                iteration_missing += 1;
                                continue;
                            }
                        }
                        iteration_found += 1;
                    }
                }
            }
        }
        
        // Push to all remotes after updating SSH config
        if !dry_run && (iteration > 1 || initial_refresh) {
            let push_errors = git_push_all(&config_dir);
            for error in &push_errors {
                warn(&format!("[sync] {}", error));
                if verbose {
                    println!("  [sync] {}", error);
                }
            }
        }
        
        found = iteration_found;
        missing = iteration_missing;
        
        // Check if we found new devices by comparing current device files with previous
        let mut current_device_files: std::collections::HashSet<String> = std::collections::HashSet::new();
        if devices_dir.exists() {
            if let Ok(entries) = fs::read_dir(&devices_dir) {
                for entry in entries {
                    if let Ok(entry) = entry {
                        if entry.path().is_file() && entry.path().extension().map_or(false, |ext| ext == "json") {
                            if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
                                current_device_files.insert(name.to_string());
                            }
                        }
                    }
                }
            }
        }
        
        // Check if new device files appeared
        for new_file in &current_device_files {
            if !previous_device_files.contains(new_file) {
                new_devices_found = true;
                break;
            }
        }
        
        // Update previous for next iteration
        previous_device_files = current_device_files;
    }
    
    (found, missing)
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

    // 2. subnet helpers
    let net_ok = net_valid(&net_prefix("192.168.43.0/24")) == Some("192.168.43".to_string())
        && net_valid(&net_prefix("192.168.43")) == Some("192.168.43".to_string())
        && net_valid("192.168.43") == Some("192.168.43".to_string())
        && net_valid("192.168.256") == None
        && net_valid("192.168") == None
        && net_valid("abc.def.ghi") == None;
    println!("subnet helpers — {}", if net_ok { "OK" } else { "FAIL" });
    ok &= net_ok;

    // 3. test config loading with sample data
    let sample_dir = PathBuf::from("/tmp/regroup-selftest");
    let _ = fs::remove_dir_all(&sample_dir);
    
    if let Err(e) = fs::create_dir_all(&sample_dir) {
        println!("temp dir creation — FAIL ({})", e);
        ok = false;
    } else {
        let devices_dir = sample_dir.join("devices");
        let _ = fs::create_dir_all(&devices_dir);
        
        let device_file = devices_dir.join("test.json");
        let device_config = r#"{"name": "test", "fingerprint": "SHA256:abc", "user": "testuser", "port": 22}"#;
        if let Err(e) = fs::write(&device_file, device_config) {
            println!("device file creation — FAIL ({})", e);
            ok = false;
        } else {
            match load_device_file(&device_file) {
                Ok(m) => {
                    let device_ok = m.name == "test" 
                        && m.fingerprint == "SHA256:abc"
                        && m.user == "testuser"
                        && m.port == 22
                        && m.port_explicit;
                    println!("device config loading — {}", if device_ok { "OK" } else { "FAIL" });
                    ok &= device_ok;
                }
                Err(e) => {
                    println!("device config loading — FAIL ({})", e);
                    ok = false;
                }
            }
        }
        
        let _ = fs::remove_dir_all(&sample_dir);
    }

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

Configuration is stored in ~/.regroup-config/ git repository where each
device has its own JSON file in the devices/ directory. Git remotes are
automatically configured for all other devices (using hostnames, not IPs).

Discovery and sync run in a loop until no new devices are found. Each device
owns its own config file and can only overwrite it. If the local device config
differs from the remote, the user is asked to resolve the conflict.

Usage:
  ./regroup                announce + listen + sync loop, update ~/.ssh/config
  ./regroup test           dry run: nothing is modified
  ./regroup -v             verbose
  ./regroup -r             refresh: pull from git remotes before sync loop
  ./regroup 192.168.43     also scan that /24 once
  ./regroup selftest       built-in checks (tools, config, config rewrite)
  ./regroup -h             this help

Config repository structure (~/.regroup-config/):
  settings.json           (optional: udp_port, wait_secs, batch_size, subnets)
  devices/
    <hostname>.json       (each device's config: name, fingerprint, user, port)

Git remotes (automatically managed using hostnames):
  user@hostname:~/.regroup-config  (for each other device)

Sync behavior:
  - Loop until no new devices are discovered
  - git pull --all on each iteration (except first)
  - git push --all after updating SSH config
  - One-way SSH connections handled gracefully
  - User prompted to resolve own config conflicts

Exit codes:
  0  all machines found
  1  at least one machine missing
  2  configuration or environment error
  3  user declined conflict resolution"
    );
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    let mut verbose = false;
    let mut dry_run = false;
    let mut help = false;
    let mut selftest = false;
    let mut net_arg: Option<String> = None;
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

    // --- configuration ---
    let conf = load_config();

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
    }

    // --- Discovery and sync loop ---
    let (found, missing) = discovery_sync_loop(&conf, &ssh_config, &local_name, &local_ip, dry_run, verbose, refresh, &net_arg);

    // Check if this device is alone (only found itself, no remotes configured)
    if !dry_run && found == 1 && missing == 0 {
        let home = match env::var("HOME") {
            Ok(h) => h,
            Err(_) => String::new(),
        };
        let config_dir = PathBuf::from(&home).join(CONFIG_DIR);
        if is_git_repo(&config_dir) {
            let remotes = git_list_remotes(&config_dir);
            if remotes.is_empty() {
                println!("\n=== Manual git sync setup ===");
                println!("This device is alone in the config. To sync with other devices:");
                println!("\nFrom this device (if you can SSH to others):");
                println!("  cd ~/.regroup-config");
                println!("  git remote add <hostname> user@hostname:~/.regroup-config");
                println!("  git push <hostname> main");
                println!("\nFrom another device (to pull this device's config):");
                println!("  cd ~/.regroup-config");
                println!("  git remote add {} {}:~/.regroup-config", local_name, local_name);
                println!("  git pull {} main", local_name);
                println!("\nNote: Authentication direction matters - you need SSH access in the direction you're syncing.");
            }
        }
    }

    println!("== {} found, {} missing ==", found, missing);
    std::process::exit(if missing == 0 { 0 } else { 1 });
}
