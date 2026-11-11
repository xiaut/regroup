//! TUI for browsing regroup connections
//!
//! View-only for now: the procedures to create and remove connections
//! (dedicated SSH key pair, installation of the public key on the target
//! device) are not implemented yet and will be redesigned (ssh-copy-id is
//! one candidate). Keys 'a' (add) and 'd' (delete) answer with an explicit
//! "not implemented" message.
//!
//! The data layer is ready: connections live in
//! `devices/<name>/connections/` under the owner-scoped naming rule (see
//! config::connection).
//!
//! This screen is not wired into the main interactive loop yet: handing
//! the terminal over to a nested ratatui loop and restoring the main
//! drawing state needs care (see ui.rs TtyUi).
#![allow(
    dead_code,
    reason = "consultation screen; wiring into the main TUI loop comes with the connection procedures"
)]

use std::io;
use std::path::Path;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use crate::config::connection::{self as conn_store, Connection, Direction as ConnDirection};
use crate::config::Device;

const NOT_IMPLEMENTED: &str =
    "Création et suppression de connexions : non implémentées pour le moment.";

/// Result of the connections screen
#[derive(Debug)]
pub enum ConnectionResult {
    /// No change
    None,
    /// Error occurred
    Error(String),
}

/// The screen currently displayed.
enum Screen {
    /// Choice of the device whose connections to browse
    DeviceList { selected: usize },
    /// All connections of the selected device (both directions)
    ConnectionList {
        device_name: String,
        items: Vec<(Connection, ConnDirection)>,
        selected: usize,
    },
    /// Details of one connection
    ConnectionDetails {
        connection: Connection,
        direction: ConnDirection,
    },
    /// Informational message (creation/removal not implemented, load error)
    Message { text: String },
}

/// Browse the connections of all known devices except the local one.
pub fn handle_connections(
    terminal: &mut ratatui::Terminal<impl ratatui::backend::Backend>,
    config_dir: &Path,
    devices: &[Device],
    local_device: &str,
) -> io::Result<ConnectionResult> {
    let devices: Vec<Device> = devices
        .iter()
        .filter(|d| d.name != local_device)
        .cloned()
        .collect();
    if devices.is_empty() {
        return Ok(ConnectionResult::Error(
            "No other devices known.".to_string(),
        ));
    }

    let mut screen = Screen::DeviceList { selected: 0 };

    loop {
        terminal.draw(|f| match &screen {
            Screen::DeviceList { selected } => {
                draw_device_list(f, &devices, *selected, local_device)
            }
            Screen::ConnectionList {
                device_name,
                items,
                selected,
            } => draw_connection_list(f, device_name, items, *selected),
            Screen::ConnectionDetails {
                connection,
                direction,
            } => draw_connection_details(f, connection, *direction),
            Screen::Message { text } => draw_message(f, text),
        })?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match &mut screen {
            Screen::DeviceList { selected } => match key.code {
                KeyCode::Char('q') => return Ok(ConnectionResult::None),
                KeyCode::Up => *selected = selected.saturating_sub(1),
                KeyCode::Down => *selected = (*selected + 1).min(devices.len() - 1),
                KeyCode::Enter => {
                    let name = devices[*selected].name.as_str().to_string();
                    screen = open_device_connections(config_dir, &name);
                }
                KeyCode::Char('a') | KeyCode::Char('d') => {
                    screen = Screen::Message {
                        text: NOT_IMPLEMENTED.to_string(),
                    };
                }
                _ => {}
            },
            Screen::ConnectionList {
                items,
                selected,
                ..
            } => match key.code {
                KeyCode::Char('q') => screen = Screen::DeviceList { selected: 0 },
                KeyCode::Up => *selected = selected.saturating_sub(1),
                KeyCode::Down => {
                    if !items.is_empty() {
                        *selected = (*selected + 1).min(items.len() - 1);
                    }
                }
                KeyCode::Enter => {
                    if !items.is_empty() {
                        let (connection, direction) = items[*selected].clone();
                        screen = Screen::ConnectionDetails {
                            connection,
                            direction,
                        };
                    }
                }
                KeyCode::Char('a') | KeyCode::Char('d') => screen = Screen::Message {
                    text: NOT_IMPLEMENTED.to_string(),
                },
                _ => {}
            },
            Screen::ConnectionDetails {
                connection,
                direction,
            } => {
                // Any quit key goes back to the device whose directory
                // holds this file: source device for an outgoing
                // connection, target device for an incoming one.
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Enter) {
                    let device = match direction {
                        ConnDirection::Outgoing => connection.from_device.clone(),
                        ConnDirection::Incoming => connection.to_device.clone(),
                    };
                    screen = open_device_connections(config_dir, &device);
                }
            }
            Screen::Message { .. } => screen = Screen::DeviceList { selected: 0 },
        }
    }
}

/// Rebuild the list of a device's connections, both directions mixed;
/// load failures and malformed files become a message screen.
fn open_device_connections(config_dir: &Path, device_name: &str) -> Screen {
    match conn_store::load_device_connections(config_dir, device_name) {
        Ok(loaded) => {
            // Malformed files are reported instead of silently hidden.
            if !loaded.warnings.is_empty() {
                return Screen::Message {
                    text: format!("Fichier(s) ignoré(s) : {}", loaded.warnings.join(" ; ")),
                };
            }
            let mut items: Vec<(Connection, ConnDirection)> = Vec::new();
            for c in loaded.outgoing {
                items.push((c, ConnDirection::Outgoing));
            }
            for c in loaded.incoming {
                items.push((c, ConnDirection::Incoming));
            }
            Screen::ConnectionList {
                device_name: device_name.to_string(),
                items,
                selected: 0,
            }
        }
        Err(e) => Screen::Message {
            text: format!("Impossible de lire les connexions de {device_name} : {e}"),
        },
    }
}

// ============================================================
// Drawing
// ============================================================

fn draw_device_list(
    f: &mut ratatui::Frame,
    devices: &[Device],
    selected: usize,
    local_device: &str,
) {
    let area = full(f.area());
    let title = format!(" Connexions — devices connues depuis {} ", local_device);

    let items: Vec<ListItem> = devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let label = format!("{} ({})", d.name, d.user);
            let style = if i == selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Span::styled(label, style))
        })
        .collect();

    render_screen(f, area, &title, Color::Cyan, items, HELP_BROWSE);
}

fn draw_connection_list(
    f: &mut ratatui::Frame,
    device_name: &str,
    items: &[(Connection, ConnDirection)],
    selected: usize,
) {
    let area = full(f.area());
    let title = format!(" Connexions de {} ", device_name);

    let entries: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, (c, d))| {
            let (dir, owner) = match d {
                ConnDirection::Outgoing => ("sortante", c.from_user.as_str()),
                ConnDirection::Incoming => ("entrante", c.to_user.as_str()),
            };
            let label = format!(
                "{} — {}, à {}",
                pretty_direction(c, *d),
                dir,
                owner
            );
            let style = if i == selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Span::styled(label, style))
        })
        .collect();

    render_screen(
        f,
        area,
        &title,
        Color::Cyan,
        entries,
        HELP_CONNECTIONS,
    );
}

fn draw_connection_details(
    f: &mut ratatui::Frame,
    connection: &Connection,
    direction: ConnDirection,
) {
    let area = full(f.area());
    let block = Block::default()
        .title(Span::styled(
            " Détails de la connexion ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL);
    f.render_widget(block, area);
    let inner = inner_of(area);

    let dir_label = match direction {
        ConnDirection::Outgoing => "sortante",
        ConnDirection::Incoming => "entrante",
    };
    let content = Paragraph::new(vec![
        Line::from(pretty_direction(connection, direction)),
        Line::from(format!("Type : {dir_label}")),
        Line::from(format!("Public key     : {}", connection.public_key)),
        Line::from(format!("Private key    : {}", connection.private_key_path)),
        Line::from(format!("Created at     : {}", connection.created_at)),
        Line::from(String::new()),
        Line::from(vec![
            Span::styled("q", Style::default().fg(Color::Yellow)),
            Span::raw(" Retour"),
        ]),
    ])
    .wrap(Wrap { trim: true });
    f.render_widget(content, inner);
}

fn draw_message(f: &mut ratatui::Frame, text: &str) {
    let area = full(f.area());
    let block = Block::default()
        .title(Span::styled(
            " Information ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL);
    f.render_widget(block, area);
    let inner = inner_of(area);
    let content = Paragraph::new(vec![
        Line::from(Span::styled(text, Style::default().fg(Color::Yellow))),
        Line::from(String::new()),
        Line::from(vec![
            Span::styled("Touche quelconque", Style::default().fg(Color::Green)),
            Span::raw(" Retour"),
        ]),
    ])
    .wrap(Wrap { trim: true });
    f.render_widget(content, inner);
}

fn pretty_direction(c: &Connection, d: ConnDirection) -> String {
    match d {
        ConnDirection::Outgoing => format!(
            "{}(ici) → {}@{}",
            c.from_user, c.to_user, c.to_device
        ),
        ConnDirection::Incoming => format!(
            "{}@{} → {}(ici)",
            c.from_user, c.from_device, c.to_user
        ),
    }
}

const HELP_BROWSE: &str = "↑/↓ Naviguer · Enter Connexions · q Retour";
const HELP_CONNECTIONS: &str =
    "↑/↓ Naviguer · Enter Détails · a Ajouter · d Supprimer · q Retour";

/// Common screen chrome: title, bordered frame, list and help line.
fn render_screen(
    f: &mut ratatui::Frame,
    area: Rect,
    title: &str,
    title_color: Color,
    items: Vec<ListItem>,
    help: &str,
) {
    f.render_widget(Clear, area);
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default().fg(title_color).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL);
    f.render_widget(block, area);
    let inner = inner_of(area);

    let list_area = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: inner.height.saturating_sub(3),
    };
    let list = List::new(items).highlight_style(Style::default().bg(Color::DarkGray));
    f.render_widget(list, list_area);

    let help_area = Rect {
        x: area.x + 1,
        y: area.y + area.height.saturating_sub(2),
        width: inner.width,
        height: 1,
    };
    f.render_widget(Paragraph::new(help), help_area);
}

fn full(area: Rect) -> Rect {
    area
}

fn inner_of(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}
