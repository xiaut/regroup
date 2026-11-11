//! TUI for browsing regroup connections
//!
//! View-only for now: the procedures to create and remove connections
//! (dedicated SSH key pair, installation of the public key on the target
//! machine) are not implemented yet and will be redesigned (ssh-copy-id is
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
use crate::config::Machine;

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
    /// Choice of the machine whose connections to browse
    MachineList { selected: usize },
    /// All connections of the selected machine (both directions)
    ConnectionList {
        machine_name: String,
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

/// Browse the connections of all known machines except the local one.
pub fn handle_connections(
    terminal: &mut ratatui::Terminal<impl ratatui::backend::Backend>,
    config_dir: &Path,
    machines: &[Machine],
    local_machine: &str,
) -> io::Result<ConnectionResult> {
    let machines: Vec<Machine> = machines
        .iter()
        .filter(|m| m.name != local_machine)
        .cloned()
        .collect();
    if machines.is_empty() {
        return Ok(ConnectionResult::Error(
            "No other machines known.".to_string(),
        ));
    }

    let mut screen = Screen::MachineList { selected: 0 };

    loop {
        terminal.draw(|f| match &screen {
            Screen::MachineList { selected } => {
                draw_machine_list(f, &machines, *selected, local_machine)
            }
            Screen::ConnectionList {
                machine_name,
                items,
                selected,
            } => draw_connection_list(f, machine_name, items, *selected),
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
            Screen::MachineList { selected } => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(ConnectionResult::None),
                KeyCode::Up => *selected = selected.saturating_sub(1),
                KeyCode::Down => *selected = (*selected + 1).min(machines.len() - 1),
                KeyCode::Enter => {
                    let name = machines[*selected].name.as_str().to_string();
                    screen = open_machine_connections(config_dir, &name);
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
                KeyCode::Esc => screen = Screen::MachineList { selected: 0 },
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
                KeyCode::Char('q') => return Ok(ConnectionResult::None),
                _ => {}
            },
            Screen::ConnectionDetails {
                connection,
                direction,
            } => {
                // Any quit key goes back to the machine whose directory
                // holds this file: source machine for an outgoing
                // connection, target machine for an incoming one.
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter) {
                    let machine = match direction {
                        ConnDirection::Outgoing => connection.from_machine.clone(),
                        ConnDirection::Incoming => connection.to_machine.clone(),
                    };
                    screen = open_machine_connections(config_dir, &machine);
                }
            }
            Screen::Message { .. } => screen = Screen::MachineList { selected: 0 },
        }
    }
}

/// Rebuild the list of a device's connections, both directions mixed;
/// load failures and malformed files become a message screen.
fn open_machine_connections(config_dir: &Path, machine_name: &str) -> Screen {
    match conn_store::load_machine_connections(config_dir, machine_name) {
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
                machine_name: machine_name.to_string(),
                items,
                selected: 0,
            }
        }
        Err(e) => Screen::Message {
            text: format!("Impossible de lire les connexions de {machine_name} : {e}"),
        },
    }
}

// ============================================================
// Drawing
// ============================================================

fn draw_machine_list(
    f: &mut ratatui::Frame,
    machines: &[Machine],
    selected: usize,
    local_machine: &str,
) {
    let area = full(f.area());
    let title = format!(" Connexions — machines connues depuis {} ", local_machine);

    let items: Vec<ListItem> = machines
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let label = format!("{} ({})", m.name, m.user);
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
    machine_name: &str,
    items: &[(Connection, ConnDirection)],
    selected: usize,
) {
    let area = full(f.area());
    let title = format!(" Connexions de {} ", machine_name);

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
            c.from_user, c.to_user, c.to_machine
        ),
        ConnDirection::Incoming => format!(
            "{}@{} → {}(ici)",
            c.from_user, c.from_machine, c.to_user
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
