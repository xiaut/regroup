//! Receive and install an SSH public key for a regroup connection
//!
//! This command is called on the TARGET device via SSH to receive a public key
//! from the SOURCE device and install it in authorized_keys.
//!
//! Usage (from source device):
//!   ssh user@target "regroup receive-install-key --source-user alice --source-device ordi --public-key '...' --connection-id alice->bob@target"

use std::env;
use std::process;

use chrono;

use crate::cli::key_management::{
    get_config_dir, install_authorized_key, commit_changes, parse_connection_id,
    get_current_user, get_local_device_name,
};
use crate::config::connection::{Connection, save_connection};

/// Arguments for receive-install-key command
#[derive(Debug)]
pub struct ReceiveInstallKeyArgs {
    pub source_user: String,
    pub source_device: String,
    pub public_key: String,
    pub connection_id: String,
}

/// Parse command line arguments for receive-install-key
pub fn parse_args() -> Result<ReceiveInstallKeyArgs, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    
    if args.is_empty() || args[0] != "receive-install-key" {
        return Err("This is not the receive-install-key command".to_string());
    }
    
    // Skip the command name
    let mut args_iter = args.into_iter().skip(1);
    
    let mut source_user: Option<String> = None;
    let mut source_device: Option<String> = None;
    let mut public_key: Option<String> = None;
    let mut connection_id: Option<String> = None;
    
    while let Some(arg) = args_iter.next() {
        match arg.as_str() {
            "--source-user" => {
                source_user = Some(args_iter.next().ok_or("--source-user requires a value".to_string())?);
            }
            "--source-device" => {
                source_device = Some(args_iter.next().ok_or("--source-device requires a value".to_string())?);
            }
            "--public-key" => {
                public_key = Some(args_iter.next().ok_or("--public-key requires a value".to_string())?);
            }
            "--connection-id" => {
                connection_id = Some(args_iter.next().ok_or("--connection-id requires a value".to_string())?);
            }
            _ => {
                if arg.starts_with("--") {
                    return Err(format!("Unknown argument: {}", arg));
                }
            }
        }
    }
    
    Ok(ReceiveInstallKeyArgs {
        source_user: source_user.ok_or("--source-user is required".to_string())?,
        source_device: source_device.ok_or("--source-device is required".to_string())?,
        public_key: public_key.ok_or("--public-key is required".to_string())?,
        connection_id: connection_id.ok_or("--connection-id is required".to_string())?,
    })
}

/// Main handler for receive-install-key command
pub fn handle_receive_install_key() -> Result<(), String> {
    let args = parse_args()?;
    
    // Validate that current user matches the target user in connection_id
    let current_user = get_current_user();
    let (from_user, to_user, to_device) = parse_connection_id(&args.connection_id)?;
    
    if current_user != to_user {
        return Err(format!(
            "Local user '{}' does not match target user '{}' in connection ID",
            current_user, to_user
        ));
    }
    
    // Validate that local device matches to_device
    let local_device = get_local_device_name();
    if local_device != to_device {
        return Err(format!(
            "Local device '{}' does not match target device '{}' in connection ID",
            local_device, to_device
        ));
    }
    
    let config_dir = get_config_dir();
    
    // Check if a connection file already exists for this incoming connection
    // Filename format for incoming: "from_user@from_device->to_user.json"
    let connections_dir = config_dir.join("devices").join(&to_device).join("connections");
    let incoming_filename = format!("{}@{}{}{}", args.source_user, args.source_device, "->", to_user);
    let incoming_connection_path = connections_dir.join(format!("{}.json", incoming_filename));
    
    if incoming_connection_path.exists() {
        return Err(format!(
            "Connection already exists for {}. Remove it first if you want to recreate it.",
            args.connection_id
        ));
    }
    
    // Create the connection object
    let connection = Connection::new(
        &args.source_user,
        &args.source_device,
        &to_user,
        &to_device,
        &args.public_key,
        &format!("/home/{}/.ssh/id_regroup_{}@{}", from_user, from_user, to_device),
        &chrono::Utc::now().to_rfc3339(),
    );
    
    // Save the connection file (on the target device, this is incoming)
    save_connection(
        &config_dir,
        &to_device,
        &connection,
        &current_user,
    ).map_err(|e| format!("Failed to save connection: {}", e))?;
    
    // Install the public key in authorized_keys
    install_authorized_key(&args.public_key, &args.connection_id)
        .map_err(|e| format!("Failed to install authorized key: {}", e))?;
    
    // Commit the changes
    let commit_message = format!("Add connection: {}@{}->{}", args.source_user, args.source_device, to_user);
    commit_changes(&config_dir, &commit_message)
        .map_err(|e| format!("Failed to commit: {}", e))?;
    
    println!("Successfully installed key for connection: {}", args.connection_id);
    Ok(())
}

/// Entry point for the receive-install-key subcommand
pub fn run() {
    match handle_receive_install_key() {
        Ok(_) => process::exit(0),
        Err(e) => {
            eprintln!("ERROR: {}", e);
            process::exit(1);
        }
    }
}
