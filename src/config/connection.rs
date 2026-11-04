//! Connection configuration
//!
//! A connection links a user on one machine to a user on another machine.
//! Each connection is stored as one file inside the `connections/`
//! directory of BOTH involved machines, each machine writing only inside
//! its own directory.
//!
//! File naming (see `Connection::filename_for`): the member belonging to
//! the machine whose directory the file lives in is written as a bare
//! user name; the other member is qualified `user@machine`. The arrow
//! carries the real direction of the connection:
//!
//! - outgoing (`from_machine` is this machine):
//!   `<from_user>-><to_user>@<to_machine>.json`
//! - incoming (`to_machine` is this machine):
//!   `<from_user>@<from_machine>-><to_user>.json`
//!
//! Modification rule: a user running regroup may only create, edit or
//! delete connection files in which their name is the member located on
//! the current machine (see `Connection::can_modify`). This is enforced
//! by the save/delete functions through the acting user.
//!
//! The create/remove procedures that call these functions are not
//! implemented yet (SSH key handling pending redesign); the storage layer
//! is complete and unit-tested.
#![allow(
    dead_code,
    reason = "data layer for the connection create/remove procedures, not implemented yet"
)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::device_dir;

/// Directory holding connection files inside a device directory.
pub const CONNECTIONS_DIR: &str = "connections";

/// Separator between the two members in a connection file name.
const ARROW: &str = "->";

/// Represents a connection between two users on two machines
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Connection {
    /// User that initiates the connection.
    pub from_user: String,
    /// Machine the connection starts from.
    pub from_machine: String,
    /// User the connection targets.
    pub to_user: String,
    /// Machine the connection ends on.
    pub to_machine: String,
    /// Public key of this connection's dedicated SSH key pair.
    pub public_key: String,
    /// Absolute path of the private key file (on the source machine).
    pub private_key_path: String,
    /// Creation timestamp (RFC3339).
    pub created_at: String,
}

impl Connection {
    pub fn new(
        from_user: &str,
        from_machine: &str,
        to_user: &str,
        to_machine: &str,
        public_key: &str,
        private_key_path: &str,
        created_at: &str,
    ) -> Self {
        Self {
            from_user: from_user.to_string(),
            from_machine: from_machine.to_string(),
            to_user: to_user.to_string(),
            to_machine: to_machine.to_string(),
            public_key: public_key.to_string(),
            private_key_path: private_key_path.to_string(),
            created_at: created_at.to_string(),
        }
    }

    /// The direction of this connection as seen from `machine`.
    pub fn direction_on(&self, machine: &str) -> Option<Direction> {
        if self.from_machine == machine {
            Some(Direction::Outgoing)
        } else if self.to_machine == machine {
            Some(Direction::Incoming)
        } else {
            None
        }
    }

    /// The user located on `machine` that this connection belongs to
    /// (source user if outgoing, target user if incoming).
    pub fn local_user(&self, machine: &str) -> Option<&str> {
        match self.direction_on(machine) {
            Some(Direction::Outgoing) => Some(&self.from_user),
            Some(Direction::Incoming) => Some(&self.to_user),
            None => None,
        }
    }

    /// Whether `acting_user` may create, edit or delete this connection
    /// file on `machine`: only the user located on that machine can.
    pub fn can_modify(&self, machine: &str, acting_user: &str) -> bool {
        self.local_user(machine) == Some(acting_user)
    }

    /// File name of this connection inside `devices/<machine>/connections/`,
    /// or None when `machine` is not involved in the connection.
    pub fn filename_for(&self, machine: &str) -> Option<String> {
        match self.direction_on(machine) {
            Some(Direction::Outgoing) => Some(format!(
                "{name}{ARROW}{to_user}@{to_machine}.json",
                name = sanitize(&self.from_user),
                to_user = sanitize(&self.to_user),
                to_machine = sanitize(&self.to_machine),
            )),
            Some(Direction::Incoming) => Some(format!(
                "{from_user}@{from_machine}{ARROW}{name}.json",
                from_user = sanitize(&self.from_user),
                from_machine = sanitize(&self.from_machine),
                name = sanitize(&self.to_user),
            )),
            None => None,
        }
    }
}

/// Direction of a connection relative to a machine
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// The connection starts from this machine.
    Outgoing,
    /// The connection ends on this machine.
    Incoming,
}

/// Characters that would break the file-name conventions.
fn sanitize(name: &str) -> String {
    name.replace('@', "_").replace('/', "_").replace(' ', "_")
}

// ============================================================
// Errors
// ============================================================

/// Errors of the connection storage operations.
#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionError {
    /// The machine is not involved in the connection; its directory must
    /// never hold a file for it.
    NotInvolved(String),
    /// The acting user is not the local member of the connection, so they
    /// may not write or delete it.
    Forbidden {
        machine: String,
        acting_user: String,
    },
    /// Underlying filesystem (or serialization) error.
    Io(String),
}

impl From<io::Error> for ConnectionError {
    fn from(e: io::Error) -> Self {
        ConnectionError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for ConnectionError {
    fn from(e: serde_json::Error) -> Self {
        ConnectionError::Io(e.to_string())
    }
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionError::NotInvolved(machine) => {
                write!(
                    f,
                    "machine {machine} is not involved in this connection"
                )
            }
            ConnectionError::Forbidden {
                machine,
                acting_user,
            } => write!(
                f,
                "user {acting_user} may not modify connections of other users on {machine}"
            ),
            ConnectionError::Io(e) => write!(f, "{e}"),
        }
    }
}

// ============================================================
// Persistence
// ============================================================

/// Directory holding a device's connection files.
pub fn connections_dir(config_dir: &Path, machine: &str) -> PathBuf {
    device_dir(config_dir, machine).join(CONNECTIONS_DIR)
}

/// Path of this connection's file inside `machine`'s directory.
pub fn connection_file_path(config_dir: &Path, machine: &str, connection: &Connection) -> Result<PathBuf, ConnectionError> {
    Ok(connections_dir(config_dir, machine)
        .join(connection.filename_for(machine).ok_or_else(|| {
            ConnectionError::NotInvolved(machine.to_string())
        })?))
}

/// Write a connection file inside `machine`'s directory.
///
/// Ownership rule: `acting_user` must be the connection member located on
/// `machine` (e.g. the connection's source user for an outgoing
/// connection). A machine therefore only ever writes inside its own
/// directory, and a user only their own connection files.
pub fn save_connection(
    config_dir: &Path,
    machine: &str,
    connection: &Connection,
    acting_user: &str,
) -> Result<(), ConnectionError> {
    let path = connection_file_path(config_dir, machine, connection)?;
    if !connection.can_modify(machine, acting_user) {
        return Err(ConnectionError::Forbidden {
            machine: machine.to_string(),
            acting_user: acting_user.to_string(),
        });
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(connection)?)?;
    Ok(())
}

/// Delete a connection file from `machine`'s directory, with the same
/// ownership rule as `save_connection`. Returns whether the file existed.
pub fn delete_connection(
    config_dir: &Path,
    machine: &str,
    connection: &Connection,
    acting_user: &str,
) -> Result<bool, ConnectionError> {
    let path = connection_file_path(config_dir, machine, connection)?;
    if !connection.can_modify(machine, acting_user) {
        return Err(ConnectionError::Forbidden {
            machine: machine.to_string(),
            acting_user: acting_user.to_string(),
        });
    }
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(&path)?;
    Ok(true)
}

/// Load a connection file, enforcing that it belongs in `machine`'s
/// directory (one of the connection members is this machine and the file
/// name matches the naming rule).
pub fn load_connection_file(path: &Path, machine: &str) -> Result<Connection, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("cannot read: {e}"))?;
    let conn: Connection =
        serde_json::from_str(&content).map_err(|e| format!("invalid JSON: {e}"))?;
    if conn.direction_on(machine).is_none() {
        return Err(format!(
            "connection {} -> {} does not involve this machine",
            conn.from_user, conn.to_machine
        ));
    }
    let expected = conn.filename_for(machine).ok_or_else(|| {
        "cannot derive the file name of a connection that does not involve this machine".to_string()
    })?;
    let actual = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if actual != expected {
        return Err(format!(
            "file name {actual} does not match the naming rule (expected {expected})"
        ));
    }
    Ok(conn)
}

/// All connections of a machine, grouped by direction. Files that cannot
/// be read, parsed or matched to the naming rule are reported in
/// `warnings` (file name + reason) instead of being silently dropped.
#[derive(Clone, Debug, Default)]
pub struct MachineConnections {
    /// Connections starting from this machine.
    pub outgoing: Vec<Connection>,
    /// Connections ending on this machine.
    pub incoming: Vec<Connection>,
    /// Unreadable/mismatching files (details for the activity log).
    pub warnings: Vec<String>,
}

/// Load every connection file in `devices/<machine>/connections/`.
pub fn load_machine_connections(
    config_dir: &Path,
    machine: &str,
) -> io::Result<MachineConnections> {
    let mut result = MachineConnections::default();
    let dir = connections_dir(config_dir, machine);
    if !dir.exists() {
        return Ok(result);
    }
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        match load_connection_file(&path, machine) {
            Ok(conn) => match conn.direction_on(machine) {
                Some(Direction::Outgoing) => result.outgoing.push(conn),
                Some(Direction::Incoming) => result.incoming.push(conn),
                None => {
                    result.warnings.push(format!(
                        "{}: connection does not involve {}",
                        path.display(),
                        machine
                    ));
                }
            },
            Err(e) => result.warnings.push(format!("{}: {e}", path.display())),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("regroup-conn-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn sample(from_user: &str, to_user: &str) -> Connection {
        Connection::new(
            from_user,
            "ordi",
            to_user,
            "laptop",
            "ssh-ed25519 AAAAkey",
            "/home/elsewhere/.ssh/id_regroup_x",
            "2026-11-05T10:00:00Z",
        )
    }

    #[test]
    fn filename_follows_the_naming_rule() {
        let conn = sample("alice", "bob");
        // On the source machine the local member is bare, the remote is qualified.
        assert_eq!(
            conn.filename_for("ordi").unwrap(),
            "alice->bob@laptop.json"
        );
        // On the target machine the arrow keeps the real direction.
        assert_eq!(
            conn.filename_for("laptop").unwrap(),
            "alice@ordi->bob.json"
        );
        assert!(conn.filename_for("phone").is_none());
    }

    #[test]
    fn direction_and_local_user() {
        let conn = sample("alice", "bob");
        assert_eq!(conn.direction_on("ordi"), Some(Direction::Outgoing));
        assert_eq!(conn.direction_on("laptop"), Some(Direction::Incoming));
        assert_eq!(conn.direction_on("phone"), None);
        assert_eq!(conn.local_user("ordi"), Some("alice"));
        assert_eq!(conn.local_user("laptop"), Some("bob"));
        assert_eq!(conn.local_user("phone"), None);
    }

    #[test]
    fn only_the_local_member_may_modify() {
        let conn = sample("alice", "bob");
        assert!(conn.can_modify("ordi", "alice"));
        assert!(!conn.can_modify("ordi", "root"));
        assert!(!conn.can_modify("ordi", "bob"));
        assert!(conn.can_modify("laptop", "bob"));
        assert!(!conn.can_modify("laptop", "alice"));
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let dir = temp_dir("roundtrip");
        let conn = sample("alice", "bob");

        // The connection is stored on both machines, each side named from
        // its own point of view.
        save_connection(&dir, "ordi", &conn, "alice").unwrap();
        save_connection(&dir, "laptop", &conn, "bob").unwrap();

        let on_ordi = load_machine_connections(&dir, "ordi").unwrap();
        assert_eq!(on_ordi.outgoing, vec![conn.clone()]);
        assert!(on_ordi.incoming.is_empty());
        assert!(on_ordi.warnings.is_empty());

        let on_laptop = load_machine_connections(&dir, "laptop").unwrap();
        assert!(on_laptop.outgoing.is_empty());
        assert_eq!(on_laptop.incoming, vec![conn.clone()]);
        assert!(on_laptop.warnings.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_rejects_other_users_and_foreign_machines() {
        let dir = temp_dir("reject");
        let conn = sample("alice", "bob");

        match save_connection(&dir, "ordi", &conn, "root").unwrap_err() {
            ConnectionError::Forbidden{ .. } => {}
            other => panic!("expected Forbidden, got {other:?}"),
        }
        match save_connection(&dir, "phone", &conn, "alice").unwrap_err() {
            ConnectionError::NotInvolved(_) => {}
            other => panic!("expected NotInvolved, got {other:?}"),
        }
        // Nothing was written: neither device directory was touched.
        assert!(!dir.join("devices/ordi/connections").exists());
        assert!(!dir.join("devices/phone").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_enforces_ownership_and_reports_existence() {
        let dir = temp_dir("delete");
        let conn = sample("alice", "bob");
        save_connection(&dir, "ordi", &conn, "alice").unwrap();

        assert!(matches!(
            delete_connection(&dir, "ordi", &conn, "root").unwrap_err(),
            ConnectionError::Forbidden { .. }
        ));
        // Forbidden delete did not remove the file.
        assert!(dir.join("devices/ordi/connections/alice->bob@laptop.json").exists());

        assert!(delete_connection(&dir, "ordi", &conn, "alice").unwrap());
        assert!(!delete_connection(&dir, "ordi", &conn, "alice").unwrap());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loader_flags_mismatching_file_names() {
        let dir = temp_dir("mismatch");
        let conn_dir = dir.join("devices/ordi/connections");
        fs::create_dir_all(&conn_dir).unwrap();
        // Wrong name: bob is not the local member of an outgoing file on ordi.
        fs::write(
            conn_dir.join("bob->alice@laptop.json"),
            serde_json::to_string_pretty(&sample("alice", "bob")).unwrap(),
        )
        .unwrap();

        let loaded = load_machine_connections(&dir, "ordi").unwrap();
        assert!(loaded.outgoing.is_empty());
        assert_eq!(loaded.warnings.len(), 1);
        assert!(loaded.warnings[0].contains("naming rule"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn users_and_machines_are_sanitized_in_file_names() {
        let conn = Connection::new(
            "we ird@user",
            "ordi",
            "bob",
            "la ptop",
            "k", "p", "t",
        );
        assert_eq!(conn.filename_for("ordi").unwrap(), "we_ird_user->bob@la_ptop.json");
    }
}
