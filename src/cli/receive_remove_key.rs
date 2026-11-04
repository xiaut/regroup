//! Receive and remove an SSH public key for a regroup connection
//!
//! This command is called on the TARGET machine via SSH to remove a public key
//! from authorized_keys and delete the connection file.
//!
//! Usage (from source machine):
//!   ssh user@target "regroup receive-remove-key --connection-id alice->bob@target"

use std::env;
use std::path::PathBuf;
use std::process;

use crate::cli::key_management::{
    get_config_dir, remove_authorized_key, commit_changes, parse_connection_id,
    get_current_user, get_local_machine_name,
};
use crate::config::connection::{load_connection_file, delete_connection};

/// Arguments for receive-remove-key command
#[derive(Debug)]
pub struct ReceiveRemoveKeyArgs {
    pub connection_id: String,
}

/// Parse command line arguments for receive-remove-key
pub fn parse_args() -> Result<ReceiveRemoveKeyArgs, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    
    if args.is_empty() || args[0] != "receive-remove-key" {
        return Err("This is not the receive-remove-key command".to_string());
    }
    
    // Skip the command name
    let mut args_iter = args.into_iter().skip(1);
    
    let mut connection_id: Option<String> = None;
    
    while let Some(arg) = args_iter.next() {
        match arg.as_str() {
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
    
    Ok(ReceiveRemoveKeyArgs {
        connection_id: connection_id.ok_or("--connection-id is required".to_string())?,
    })
}

/// Main handler for receive-remove-key command
pub fn handle_receive_remove_key() -> Result<(), String> {
    let args = parse_args()?;
    
    // Parse connection ID to get components
    // connection_id format: "from_user->to_user@to_machine"
    let (from_user, to_user, to_machine) = parse_connection_id(&args.connection_id)?;
    
    // Validate that current user matches the target user
    let current_user = get_current_user();
    if current_user != to_user {
        return Err(format!(
            "Local user '{}' does not match target user '{}' in connection ID",
            current_user, to_user
        ));
    }
    
    // Validate that local machine matches to_machine
    let local_machine = get_local_machine_name();
    if local_machine != to_machine {
        return Err(format!(
            "Local machine '{}' does not match target machine '{}' in connection ID",
            local_machine, to_machine
        ));
    }
    
    let config_dir = get_config_dir();
    let connections_dir = config_dir.join("devices").join(&local_machine).join("connections");
    
    // Find the connection file that matches from_user and to_user
    // On target machine, incoming connection files are named: from_user@from_machine->to_user.json
    // We need to find the file where the local user is to_user
    let mut matching_file: Option<PathBuf> = None;
    if connections_dir.exists() {
        for entry in std::fs::read_dir(&connections_dir).map_err(|e| format!("Cannot read connections directory: {}", e))? {
            let entry = entry.map_err(|e| format!("Cannot read directory entry: {}", e))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Filename format: from_user@from_machine->to_user.json
            if filename.ends_with(".json") {
                let base_name = &filename[..filename.len()-5]; // Remove .json
                let parts: Vec<&str> = base_name.split("->").collect();
                if parts.len() == 2 {
                    let from_part = parts[0];
                    let to_part = parts[1];
                    
                    // Check if this connection is incoming (to_part should be our to_user)
                    if to_part == to_user {
                        // Also check from_user
                        let from_parts: Vec<&str> = from_part.split('@').collect();
                        if from_parts.len() == 2 && from_parts[0] == from_user {
                            matching_file = Some(path);
                            break;
                        }
                    }
                }
            }
        }
    }
    
    let connection_path = matching_file.ok_or(format!(
        "No matching connection file found for {}->{}@{}",
        from_user, to_user, to_machine
    ))?;
    
    if !connection_path.exists() {
        return Err(format!("Connection file not found: {}", connection_path.display()));
    }
    
    // Load the connection to get the public key
    let connection = load_connection_file(&connection_path, &local_machine)
        .map_err(|e| format!("Failed to load connection: {}", e))?;
    
    // Delete the connection file
    delete_connection(
        &config_dir,
        &local_machine,
        &connection,
        &current_user,
    ).map_err(|e| format!("Failed to delete connection: {}", e))?;
    
    // Remove the public key from authorized_keys
    remove_authorized_key(&connection.public_key)
        .map_err(|e| format!("Failed to remove authorized key: {}", e))?;
    
    // Commit the changes
    let commit_message = format!("Remove connection: {}@{}->{}", from_user, connection.from_machine, to_user);
    commit_changes(&config_dir, &commit_message)
        .map_err(|e| format!("Failed to commit: {}", e))?;
    
    println!("Successfully removed key for connection: {}", args.connection_id);
    Ok(())
}

/// Entry point for the receive-remove-key subcommand
pub fn run() {
    match handle_receive_remove_key() {
        Ok(_) => process::exit(0),
        Err(e) => {
            eprintln!("ERROR: {}", e);
            process::exit(1);
        }
    }
}
