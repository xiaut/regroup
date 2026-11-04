//! TUI for managing regroup connections
//!
//! This module provides the interactive interface for:
//! - Creating new connections between machines
//! - Viewing existing connections
//! - Removing connections

use std::io::{self, Write};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::config::{Machine, Connection, CONNECTIONS_DIR};
use crate::ssh::key_manager;
use std::path::Path;

/// State for connection management TUI
pub enum ConnectionState {
    /// List of machines to select target
    MachineList {
        machines: Vec<Machine>,
        selected: Option<usize>,
        local_machine: String,
        local_user: String,
    },
    /// List of connections for the selected machine
    ConnectionList {
        machine_name: String,
        connections: Vec<Connection>,
        selected: Option<usize>,
        local_machine: String,
        is_outgoing: bool, // true = outgoing, false = incoming
    },
    /// Confirmation before creating a connection
    ConfirmCreate {
        local_machine: String,
        local_user: String,
        target_machine: String,
        target_user: String,
    },
    /// Confirmation before removing a connection
    ConfirmRemove {
        local_machine: String,
        target_machine: String,
        connection: Connection,
    },
    /// Display connection details
    ConnectionDetails {
        connection: Connection,
        local_machine: String,
    },
}

/// Result of connection management
pub enum ConnectionResult {
    /// A new connection was created
    Created(Connection),
    /// A connection was removed
    Removed(Connection),
    /// No change
    None,
    /// Error occurred
    Error(String),
}

/// Manages connection TUI for the local machine
pub fn handle_connections<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    config_dir: &Path,
    machines: &[Machine],
    local_machine: &str,
    local_user: &str,
) -> io::Result<ConnectionResult> {
    // Find the local machine in the list
    let local_machine_index = machines.iter()
        .position(|m| m.name == local_machine)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("Local machine {} not found", local_machine)))?;
    
    let local_machine_obj = &machines[local_machine_index];
    
    // Filter out the local machine from the target list
    let target_machines: Vec<Machine> = machines.iter()
        .filter(|m| m.name != local_machine)
        .cloned()
        .collect();
    
    if target_machines.is_empty() {
    }
    
    
    let mut state = ConnectionState::MachineList {
        machines: target_machines,
        selected: None,
        local_machine: local_machine.to_string(),
        local_user: local_user.to_string(),
    };
    
    loop {
        terminal.draw(|f| {
            match &state {
                ConnectionState::MachineList { machines, selected, local_machine, local_user } => {
                    draw_machine_list(f, machines, *selected, local_machine, local_user);
                }
                ConnectionState::ConnectionList { machine_name, connections, selected, local_machine, is_outgoing } => {
                    draw_connection_list(f, machine_name, connections, *selected, local_machine, *is_outgoing);
                }
                ConnectionState::ConfirmCreate { local_machine, local_user, target_machine, target_user } => {
                    draw_confirm_create(f, local_machine, local_user, target_machine, target_user);
                }
                ConnectionState::ConfirmRemove { local_machine, target_machine, connection } => {
                    draw_confirm_remove(f, local_machine, target_machine, connection);
                }
                ConnectionState::ConnectionDetails { connection, local_machine } => {
                    draw_connection_details(f, connection, local_machine);
                }
            }
        })?;
        
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match &mut state {
                    ConnectionState::MachineList { machines, selected, local_machine, local_user } => {
                        match key.code {
                            KeyCode::Esc | KeyCode::Char('q') => return Ok(ConnectionResult::None),
                            KeyCode::Up => {
                                if let Some(sel) = *selected {
                                    *selected = Some(sel.saturating_sub(1));
                                } else if !machines.is_empty() {
                                    *selected = Some(0);
                                }
                            }
                            KeyCode::Down => {
                                if let Some(sel) = *selected {
                                    if sel < machines.len() - 1 {
                                        *selected = Some(sel + 1);
                                    }
                                } else if !machines.is_empty() {
                                    *selected = Some(0);
                                }
                            }
                            KeyCode::Enter => {
                                if let Some(index) = *selected {
                                    let target = &machines[index];
                                    // Show connection list for this target
                                    // First, try to load existing connections
                                    match key_manager::list_connections_for_target(config_dir, &target.name) {
                                        Ok(conns) => {
                                            state = ConnectionState::ConnectionList {
                                                machine_name: target.name.clone(),
                                                connections: conns,
                                                selected: None,
                                                local_machine: local_machine.clone(),
                                                is_outgoing: true,
                                            };
                                        }
                                        Err(_) => {
                                            // No connections yet, show empty list
                                            state = ConnectionState::ConnectionList {
                                                machine_name: target.name.clone(),
                                                connections: Vec::new(),
                                                selected: None,
                                                local_machine: local_machine.clone(),
                                                is_outgoing: true,
                                            };
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('a') => {
                                if let Some(index) = *selected {
                                    let target = &machines[index];
                                    state = ConnectionState::ConfirmCreate {
                                        local_machine: local_machine.clone(),
                                        local_user: local_user.clone(),
                                        target_machine: target.name.clone(),
                                        target_user: target.user.clone(),
                                    };
                                }
                            }
                            _ => {}
                        }
                    }
                    ConnectionState::ConnectionList { machine_name, connections, selected, local_machine, is_outgoing } => {
                        match key.code {
                            KeyCode::Esc => {
                                // Return to machine list
                                state = ConnectionState::MachineList {
                                    machines: machines.to_vec(),
                                    selected: None,
                                    local_machine: local_machine.clone(),
                                    local_user: local_user.to_string(),
                                };
                            }
                            KeyCode::Up => {
                                if let Some(sel) = *selected {
                                    *selected = Some(sel.saturating_sub(1));
                                } else if !connections.is_empty() {
                                    *selected = Some(0);
                                }
                            }
                            KeyCode::Down => {
                                if let Some(sel) = *selected {
                                    if sel < connections.len() - 1 {
                                        *selected = Some(sel + 1);
                                    }
                                } else if !connections.is_empty() {
                                    *selected = Some(0);
                                }
                            }
                            KeyCode::Enter => {
                                if let Some(index) = *selected {
                                    state = ConnectionState::ConnectionDetails {
                                        connection: connections[index].clone(),
                                        local_machine: local_machine.clone(),
                                    };
                                }
                            }
                            KeyCode::Char('d') => {
                                if let Some(index) = *selected {
                                    state = ConnectionState::ConfirmRemove {
                                        local_machine: local_machine.clone(),
                                        target_machine: machine_name.clone(),
                                        connection: connections[index].clone(),
                                    };
                                }
                            }
                            KeyCode::Char('a') => {
                                // Add new connection to this machine
                                // We need to find the machine object
                                if let Some(target) = machines.iter().find(|m| m.name == *machine_name) {
                                    state = ConnectionState::ConfirmCreate {
                                        local_machine: local_machine.clone(),
                                        local_user: local_user.to_string(),
                                        target_machine: target.name.clone(),
                                        target_user: target.user.clone(),
                                    };
                                }
                            }
                            _ => {}
                        }
                    }
                    ConnectionState::ConfirmCreate { local_machine, local_user, target_machine, target_user } => {
                        match key.code {
                            KeyCode::Esc | KeyCode::Char('n') => {
                                // Return to machine list
                                state = ConnectionState::MachineList {
                                    machines: machines.to_vec(),
                                    selected: None,
                                    local_machine: local_machine.clone(),
                                    local_user: local_user.clone(),
                                };
                            }
                            KeyCode::Char('y') | KeyCode::Enter => {
                                // Create the connection
                                return match key_manager::create_connection(
                                    config_dir,
                                    local_machine,
                                    local_user,
                                    target_machine,
                                    target_user,
                                    "ed25519",
                                ) {
                                    Ok(conn) => {
                                        // Install the key on the target machine
                                        if let Err(e) = key_manager::install_public_key(
                                            target_machine,
                                            22, // Default port, should be looked up
                                            target_user,
                                            &conn.public_key,
                                        ) {
                                            return Ok(ConnectionResult::Error(format!("Failed to install key: {}", e)));
                                        }
                                        Ok(ConnectionResult::Created(conn))
                                    }
                                    Err(e) => Ok(ConnectionResult::Error(format!("Failed to create connection: {}", e))),
                                };
                            }
                            _ => {}
                        }
                    }
                    ConnectionState::ConfirmRemove { local_machine, target_machine, connection } => {
                        match key.code {
                            KeyCode::Esc | KeyCode::Char('n') => {
                                // Return to connection list
                                match key_manager::list_connections_for_target(config_dir, target_machine) {
                                    Ok(conns) => {
                                        state = ConnectionState::ConnectionList {
                                            machine_name: target_machine.clone(),
                                            connections: conns,
                                            selected: None,
                                            local_machine: local_machine.clone(),
                                            is_outgoing: true,
                                        };
                                    }
                                    Err(_) => {
                                        state = ConnectionState::ConnectionList {
                                            machine_name: target_machine.clone(),
                                            connections: Vec::new(),
                                            selected: None,
                                            local_machine: local_machine.clone(),
                                            is_outgoing: true,
                                        };
                                    }
                                }
                            }
                            KeyCode::Char('y') | KeyCode::Enter => {
                                // Remove the connection
                                return match key_manager::remove_connection(
                                    config_dir,
                                    local_machine,
                                    target_machine,
                                    connection,
                                ) {
                                    Ok(_) => Ok(ConnectionResult::Removed(connection.clone())),
                                    Err(e) => Ok(ConnectionResult::Error(format!("Failed to remove connection: {}", e))),
                                };
                            }
                            _ => {}
                        }
                    }
                    ConnectionState::ConnectionDetails { connection, local_machine } => {
                        match key.code {
                            KeyCode::Esc | KeyCode::Char('q') => {
                                // Return to connection list
                                // We need to reload the connections for the target machine
                                let target_machine = connection.to.split('@').last().unwrap_or("");
                                match key_manager::list_connections_for_target(config_dir, target_machine) {
                                    Ok(conns) => {
                                        state = ConnectionState::ConnectionList {
                                            machine_name: target_machine.to_string(),
                                            connections: conns,
                                            selected: None,
                                            local_machine: local_machine.clone(),
                                            is_outgoing: true,
                                        };
                                    }
                                    Err(_) => {
                                        state = ConnectionState::ConnectionList {
                                            machine_name: target_machine.to_string(),
                                            connections: Vec::new(),
                                            selected: None,
                                            local_machine: local_machine.clone(),
                                            is_outgoing: true,
                                        };
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

/// Draws the machine list
fn draw_machine_list(
    f: &mut ratatui::Frame,
    machines: &[Machine],
    selected: Option<usize>,
    local_machine: &str,
    local_user: &str,
) {
    let area = f.size();
    
    let title = format!(" Connexions depuis {} ({}) ", local_machine, local_user);
    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL);
    f.render_widget(block, area);
    
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    
    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("↑/↓", Style::default().fg(Color::Green)),
        Span::raw(" Naviguer | "),
        Span::styled("a", Style::default().fg(Color::Green)),
        Span::raw(" Ajouter | "),
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::raw(" Retour"),
    ]));
    f.render_widget(help, inner);
    
    // Machine list
    let list_area = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: inner.height.saturating_sub(3),
    };
    
    let items: Vec<ListItem> = machines.iter()
        .enumerate()
        .map(|(i, machine)| {
            let content = if i == selected.unwrap_or(usize::MAX) {
                Span::styled(
                    format!("{} ({})", machine.name, machine.user),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(
                    format!("{} ({})", machine.name, machine.user),
                    Style::default(),
                )
            };
            ListItem::new(content)
        })
        .collect();
    
    let list = List::new(items)
        .block(Block::default())
        .highlight_style(Style::default().bg(Color::DarkGray));
    f.render_widget(list, list_area);
}

/// Draws the connection list
fn draw_connection_list(
    f: &mut ratatui::Frame,
    machine_name: &str,
    connections: &[Connection],
    selected: Option<usize>,
    local_machine: &str,
    is_outgoing: bool,
) {
    let area = f.size();
    
    let connection_type = if is_outgoing { "sortantes" } else { "entrantes" };
    let title = format!(" Connexions {} vers {} ", connection_type, machine_name);
    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL);
    f.render_widget(block, area);
    
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    
    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("↑/↓", Style::default().fg(Color::Green)),
        Span::raw(" Naviguer | "),
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::raw(" Détails | "),
        Span::styled("a", Style::default().fg(Color::Green)),
        Span::raw(" Ajouter | "),
        Span::styled("d", Style::default().fg(Color::Red)),
        Span::raw(" Supprimer | "),
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::raw(" Retour"),
    ]));
    f.render_widget(help, inner);
    
    // Connection list
    let list_area = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: inner.height.saturating_sub(3),
    };
    
    let items: Vec<ListItem> = connections.iter()
        .enumerate()
        .map(|(i, conn)| {
            // Extract short key fingerprint for display
            let key_short = if conn.public_key.len() > 30 {
                &conn.public_key[..30]
            } else {
                &conn.public_key
            };
            
            let content = if i == selected.unwrap_or(usize::MAX) {
                Span::styled(
                    format!("{} → {} ({}...)", conn.from, conn.to, key_short),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(
                    format!("{} → {} ({}...)", conn.from, conn.to, key_short),
                    Style::default(),
                )
            };
            ListItem::new(content)
        })
        .collect();
    
    let list = List::new(items)
        .block(Block::default())
        .highlight_style(Style::default().bg(Color::DarkGray));
    f.render_widget(list, list_area);
}

/// Draws the create connection confirmation
fn draw_confirm_create(
    f: &mut ratatui::Frame,
    local_machine: &str,
    local_user: &str,
    target_machine: &str,
    target_user: &str,
) {
    let area = f.size();
    
    let title = " Créer une nouvelle connexion ";
    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL);
    f.render_widget(block, area);
    
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    
    let content = Paragraph::new(vec![
        Line::from(vec![
            Span::raw("De: "),
            Span::styled(format!("{}@{}", local_user, local_machine), Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::raw("À: "),
            Span::styled(format!("{}@{}", target_user, target_machine), Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![]),
        Line::from(vec![
            Span::raw("Une nouvelle paire de clés SSH sera générée."),
        ]),
        Line::from(vec![
            Span::raw("La clé publique sera installée sur la machine cible."),
        ]),
        Line::from(vec![]),
        Line::from(vec![
            Span::styled("y", Style::default().fg(Color::Green)),
            Span::raw(" Oui | "),
            Span::styled("n", Style::default().fg(Color::Red)),
            Span::raw(" Non"),
        ]),
    ]);
    f.render_widget(content, inner);
}

/// Draws the remove connection confirmation
fn draw_confirm_remove(
    f: &mut ratatui::Frame,
    local_machine: &str,
    target_machine: &str,
    connection: &Connection,
) {
    let area = f.size();
    
    let title = " Supprimer la connexion ";
    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL);
    f.render_widget(block, area);
    
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    
    let content = Paragraph::new(vec![
        Line::from(vec![
            Span::raw("Supprimer: "),
            Span::styled(format!("{} → {}", connection.from, connection.to), Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![]),
        Line::from(vec![
            Span::raw("La clé SSH sera supprimée des deux machines."),
        ]),
        Line::from(vec![]),
        Line::from(vec![
            Span::styled("y", Style::default().fg(Color::Green)),
            Span::raw(" Oui | "),
            Span::styled("n", Style::default().fg(Color::Red)),
            Span::raw(" Non"),
        ]),
    ]);
    f.render_widget(content, inner);
}

/// Draws connection details
fn draw_connection_details(
    f: &mut ratatui::Frame,
    connection: &Connection,
    local_machine: &str,
) {
    let area = f.size();
    
    let title = " Détails de la connexion ";
    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL);
    f.render_widget(block, area);
    
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    
    let content = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("De: ", Style::default().fg(Color::Green)),
            Span::raw(&connection.from),
        ]),
        Line::from(vec![
            Span::styled("À: ", Style::default().fg(Color::Green)),
            Span::raw(&connection.to),
        ]),
        Line::from(vec![]),
        Line::from(vec![
            Span::styled("Clé publique: ", Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::raw(&connection.public_key),
        ]),
        Line::from(vec![]),
        Line::from(vec![
            Span::styled("Clé privée: ", Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::raw(&connection.private_key_path),
        ]),
        Line::from(vec![]),
        Line::from(vec![
            Span::styled("Créée le: ", Style::default().fg(Color::Green)),
            Span::raw(&connection.created_at),
        ]),
        Line::from(vec![]),
        Line::from(vec![
            Span::styled("q", Style::default().fg(Color::Yellow)),
            Span::raw(" Retour"),
        ]),
    ]).wrap(Wrap { trim: true });
    f.render_widget(content, inner);
}
