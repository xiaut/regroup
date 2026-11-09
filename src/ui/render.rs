//! UI rendering functions and styling utilities.
//!
//! Contains all the rendering logic for the TUI interface, including machine
//! panels, activity log, help panel, and question block rendering.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};

use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Cell, Paragraph, Row, Table};

use super::logging::{BLINK_MS, KEY_FLASH_MS, PULSE_MS};
use super::types::{ActivePane, Level};

/// Minimum terminal width to display machine panels side-by-side.
/// Below this, panels stack vertically.
pub const MIN_WIDTH_FOR_SIDE_BY_SIDE: u16 = 80;

/// Static look of one machine panel: title, border color and the status
/// shown when the machine has no animated message.
pub struct PanelTheme {
    pub title: &'static str,
    pub border: Color,
    pub idle_status: &'static str,
}

impl PanelTheme {
    pub fn new(title: &'static str, border: Color, idle_status: &'static str) -> Self {
        Self {
            title,
            border,
            idle_status,
        }
    }
}

/// A machine's animated status, shown in the last column of its row.
pub struct RowFlash {
    pub level: Level,
    pub msg: String,
    pub start: Instant,
}

/// A key press, identified by how it appears in the hint line's `[...]`
/// tokens (`s`, `Enter`, `Dn`, ...).
pub struct KeyFlash {
    pub key: String,
    pub start: Instant,
}

/// What the question block needs to render, besides the question itself.
pub struct QuestionCtx<'a> {
    /// Text typed for the rename question.
    pub input: &'a str,
    /// Machines waiting for a decision (drives the Idle hints).
    pub pending: usize,
    /// Whether the help panel is shown.
    pub help: bool,
    /// Verbose mode: mentioned in the hints.
    pub verbose: bool,
    /// Which pane is currently active.
    pub active_pane: super::types::ActivePane,
    /// Last key press, highlighted in the hint line while recent.
    pub key_flash: Option<&'a KeyFlash>,
}

/// A machine panel: one row per machine, with the animated per-machine
/// status in the last column. `selected` highlights a row (selection in
/// the Detected panel, delete mode in the Added panel). A recent beacon
/// pulses the row: bright background fading back to the base color.
pub fn render_machines(
    frame: &mut Frame,
    area: Rect,
    theme: PanelTheme,
    devices: &[super::types::Device],
    selected: Option<usize>,
    flashes: &HashMap<String, RowFlash>,
    pulses: &HashMap<String, Instant>,
) {
    let now = Instant::now();
    let pulse_duration = Duration::from_millis(PULSE_MS);
    let rows = devices.iter().enumerate().map(|(i, d)| {
        let flash = flashes
            .get(&d.key)
            .filter(|f| now.duration_since(f.start) < super::logging::flash_duration(f.level));
        let pulse = pulses
            .get(&d.key)
            .map(|t| now.duration_since(*t))
            .filter(|e| *e < pulse_duration);
        let status = if let Some(f) = flash {
            f.msg.clone()
        } else if pulse.is_some() {
            "beacon received".to_string()
        } else {
            theme.idle_status.to_string()
        };
        let style = row_style(theme.border, selected == Some(i), flash, pulse, now);
        Row::new([
            Cell::from(d.name.clone()),
            Cell::from(d.ip.clone().unwrap_or_else(|| "-".to_string())),
            Cell::from(d.port.to_string()),
            Cell::from(status),
        ])
        .style(style)
    });
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(35),
            Constraint::Percentage(30),
            Constraint::Length(5),
            Constraint::Min(8),
        ],
    )
    .header(Row::new(["Name", "IP", "Port", "Status"]).style(Style::new().bold()))
    .block(
        Block::bordered()
            .title(format!("{} — {}", theme.title, devices.len()))
            .border_style(Style::new().fg(theme.border)),
    );
    frame.render_widget(table, area);
}

/// Style of a machine row: the panel's base color, overridden by the
/// selection, an urgent blink (warning, security), a beacon pulse, or the
/// row's animated status, in that order of precedence.
pub fn row_style(
    base: Color,
    selected: bool,
    flash: Option<&RowFlash>,
    pulse: Option<Duration>,
    now: Instant,
) -> Style {
    if selected {
        return Style::new().add_modifier(Modifier::REVERSED);
    }
    // Urgent statuses keep their blink so they can never be masked.
    if let Some(f) = flash.filter(|f| matches!(f.level, Level::Warning | Level::Security)) {
        return flash_style(f, blink(now.duration_since(f.start)));
    }
    // Beacon received: the row lights up, then fades back to its base
    // color through three phases, so the user sees the app receiving.
    if let Some(elapsed) = pulse {
        let third = Duration::from_millis(PULSE_MS / 3);
        if elapsed < third {
            return Style::new().fg(Color::Black).bg(Color::Cyan);
        }
        if elapsed < third * 2 {
            return Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
        }
        return Style::new().fg(base).add_modifier(Modifier::BOLD);
    }
    match flash {
        Some(f) => flash_style(f, blink(now.duration_since(f.start))),
        None => Style::new().fg(base),
    }
}

/// Style of a row's animated status, by level (`on` is the visible phase
/// of the blinking levels).
pub fn flash_style(f: &RowFlash, on: bool) -> Style {
    match f.level {
        Level::Info => Style::new().fg(Color::Cyan),
        Level::Success => Style::new().fg(Color::Black).bg(Color::Green),
        Level::Warning => {
            if on {
                Style::new().fg(Color::Black).bg(Color::Yellow)
            } else {
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            }
        }
        Level::Security => {
            if on {
                Style::new().fg(Color::White).bg(Color::Red)
            } else {
                Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
            }
        }
    }
}

/// Alternating on/off phase of a blinking row status.
pub fn blink(elapsed: Duration) -> bool {
    elapsed.as_millis() / BLINK_MS as u128 % 2 == 0
}

/// How a pressed key appears in the hint line's `[...]` tokens (`s`,
/// `Enter`, `Dn`, ...), or `None` when it appears in none of them.
pub fn key_label(code: KeyCode) -> Option<String> {
    match code {
        KeyCode::Char(c) => Some(c.to_string()),
        KeyCode::Enter => Some("Enter".to_string()),
        KeyCode::Esc => Some("Esc".to_string()),
        KeyCode::Up => Some("Up".to_string()),
        KeyCode::Down => Some("Dn".to_string()),
        KeyCode::PageUp => Some("PgUp".to_string()),
        KeyCode::PageDown => Some("PgDn".to_string()),
        _ => None,
    }
}

/// A hint line with its `[key]` tokens split into spans; the token holding
/// the last pressed key is highlighted for a short moment (bright first,
/// then bold only), so the user sees the app react to the key press.
pub fn hint_spans(text: &str, key_flash: Option<&KeyFlash>) -> Line<'static> {
    let Some(flash) = key_flash else {
        return Line::from(text.to_string());
    };
    let elapsed = Instant::now().duration_since(flash.start);
    if elapsed >= Duration::from_millis(KEY_FLASH_MS) {
        return Line::from(text.to_string());
    }
    let lit = || -> Style {
        // Lit only while recent, then a fading tail so the effect decays
        // instead of switching off in one frame.
        if elapsed < Duration::from_millis(KEY_FLASH_MS / 2) {
            Style::new()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::BOLD)
        }
    };
    let mut spans: Vec<Span> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let Some(end) = rest[start..].find(']') else {
            break;
        };
        let end = start + end;
        if start > 0 {
            spans.push(Span::from(rest[..start].to_string()));
        }
        let token = &rest[start..=end];
        let inner = &rest[start + 1..end];
        let style = if inner.split('/').any(|k| k == flash.key) {
            lit()
        } else {
            Style::new()
        };
        spans.push(Span::styled(token.to_string(), style));
        rest = &rest[end + 1..];
    }
    spans.push(Span::from(rest.to_string()));
    Line::from(spans)
}

/// Logs pane: bottom-anchored log, shifted up by `scroll` lines. While
/// verbose mode is on, the pane's title says so.
pub fn render_log(frame: &mut Frame, area: Rect, log: &[String], scroll: u16, verbose: bool) {
    let visible = area.height.saturating_sub(2) as usize;
    let skip = log
        .len()
        .saturating_sub(visible + scroll as usize)
        .min(log.len());
    let lines: Vec<Line> = log
        .iter()
        .map(|msg| Line::styled(msg.clone(), log_style(msg)))
        .collect();
    let title = if verbose { "Logs — verbose" } else { "Logs" };
    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::bordered().title(title))
        .scroll((skip as u16, 0));
    frame.render_widget(paragraph, area);
}

/// Help panel (key '?'): the CLI help text, replacing the machine panels
/// and the activity pane while shown. Section headers are bold; the pane
/// is bottom-anchored, shifted up by `scroll` lines.
pub fn render_help(frame: &mut Frame, area: Rect, scroll: u16) {
    let lines: Vec<Line> = crate::cli::usage_text()
        .lines()
        .map(|l| {
            if l.ends_with(':') {
                Line::styled(l.to_string(), Style::new().bold())
            } else {
                Line::from(l.to_string())
            }
        })
        .collect();
    let total = lines.len();
    let visible = area.height.saturating_sub(2) as usize;
    let skip = total.saturating_sub(visible + scroll as usize).min(total);
    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::bordered().title("Help"))
        .scroll((skip as u16, 0));
    frame.render_widget(paragraph, area);
}

/// Color of a log line, derived from its content.
pub fn log_style(msg: &str) -> Style {
    if msg.contains("SECURITY") || msg.contains("ERROR") {
        Style::new().fg(Color::Red)
    } else if msg.starts_with("WARNING") || msg.starts_with("  !") {
        Style::new().fg(Color::Yellow)
    } else if msg.starts_with("[~]") {
        Style::new().fg(Color::Cyan)
    } else if msg.starts_with("[DRY RUN]") {
        Style::new().fg(Color::Magenta)
    } else if msg.starts_with("  +") {
        Style::new().fg(Color::Green)
    } else {
        Style::new()
    }
}

/// Render all three panes: Detected and Added side-by-side on top, Logs below.
/// The active pane gets a bold border. All panes are always visible.
pub fn render_all_panes(
    frame: &mut Frame,
    area: Rect,
    active_pane: ActivePane,
    detected: &[super::types::Device],
    detected_selected: Option<usize>,
    added: &[super::types::Device],
    added_selected: Option<usize>,
    log: &[String],
    log_scroll: u16,
    verbose: bool,
    flashes: &HashMap<String, RowFlash>,
    pulses: &HashMap<String, Instant>,
) {
    let width = area.width;
    let height = area.height;

    // Minimum width for side-by-side top panes
    let min_width_for_side_by_side = 80;

    if width >= min_width_for_side_by_side {
        // Layout: Detected | Added (top row), Logs (bottom row)
        let top_height = height / 2;
        let logs_height = height - top_height;

        let [top_area, logs_area] = Layout::vertical([
            Constraint::Length(top_height),
            Constraint::Length(logs_height),
        ])
        .areas(area);

        // Split top area into Detected (left) and Added (right)
        let detected_width = width / 2;
        let added_width = width - detected_width;

        let [detected_area, added_area] = Layout::horizontal([
            Constraint::Length(detected_width),
            Constraint::Length(added_width),
        ])
        .areas(top_area);

        // Render Detected pane
        let detected_theme = PanelTheme::new("Detected", Color::Yellow, "detected");
        let detected_border_style = if matches!(active_pane, ActivePane::Detected) {
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Color::Yellow)
        };
        let detected_border_type = if matches!(active_pane, ActivePane::Detected) {
            BorderType::Double
        } else {
            BorderType::Plain
        };

        let detected_rows = detected.iter().enumerate().map(|(i, d)| {
            let now = Instant::now();
            let pulse_duration = Duration::from_millis(PULSE_MS);
            let flash = flashes
                .get(&d.key)
                .filter(|f| now.duration_since(f.start) < super::logging::flash_duration(f.level));
            let pulse = pulses
                .get(&d.key)
                .map(|t| now.duration_since(*t))
                .filter(|e| *e < pulse_duration);
            let status = if let Some(f) = flash {
                f.msg.clone()
            } else if pulse.is_some() {
                "beacon received".to_string()
            } else {
                detected_theme.idle_status.to_string()
            };
            let style = row_style(
                detected_theme.border,
                detected_selected == Some(i),
                flash,
                pulse,
                now,
            );
            Row::new([
                Cell::from(d.name.clone()),
                Cell::from(d.ip.clone().unwrap_or_else(|| "-".to_string())),
                Cell::from(d.port.to_string()),
                Cell::from(status),
            ])
            .style(style)
        });
        let detected_table = Table::new(
            detected_rows,
            [
                Constraint::Percentage(35),
                Constraint::Percentage(30),
                Constraint::Length(5),
                Constraint::Min(8),
            ],
        )
        .header(Row::new(["Name", "IP", "Port", "Status"]).style(Style::new().bold()))
        .block(
            Block::bordered()
                .title(format!("{} — {}", detected_theme.title, detected.len()))
                .border_type(detected_border_type)
                .border_style(detected_border_style),
        );
        frame.render_widget(detected_table, detected_area);

        // Render Added pane
        let added_theme = PanelTheme::new("Added", Color::Green, "known");
        let added_border_style = if matches!(active_pane, ActivePane::Added) {
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Color::Green)
        };
        let added_border_type = if matches!(active_pane, ActivePane::Added) {
            BorderType::Double
        } else {
            BorderType::Plain
        };

        let added_rows = added.iter().enumerate().map(|(i, d)| {
            let now = Instant::now();
            let pulse_duration = Duration::from_millis(PULSE_MS);
            let flash = flashes
                .get(&d.key)
                .filter(|f| now.duration_since(f.start) < super::logging::flash_duration(f.level));
            let pulse = pulses
                .get(&d.key)
                .map(|t| now.duration_since(*t))
                .filter(|e| *e < pulse_duration);
            let status = if let Some(f) = flash {
                f.msg.clone()
            } else if pulse.is_some() {
                "beacon received".to_string()
            } else {
                added_theme.idle_status.to_string()
            };
            let style = row_style(
                added_theme.border,
                added_selected == Some(i),
                flash,
                pulse,
                now,
            );
            Row::new([
                Cell::from(d.name.clone()),
                Cell::from(d.ip.clone().unwrap_or_else(|| "-".to_string())),
                Cell::from(d.port.to_string()),
                Cell::from(status),
            ])
            .style(style)
        });
        let added_table = Table::new(
            added_rows,
            [
                Constraint::Percentage(35),
                Constraint::Percentage(30),
                Constraint::Length(5),
                Constraint::Min(8),
            ],
        )
        .header(Row::new(["Name", "IP", "Port", "Status"]).style(Style::new().bold()))
        .block(
            Block::bordered()
                .title(format!("{} — {}", added_theme.title, added.len()))
                .border_type(added_border_type)
                .border_style(added_border_style),
        );
        frame.render_widget(added_table, added_area);

        // Render Logs pane
        let logs_title = if verbose { "Logs — verbose" } else { "Logs" };
        let logs_border_style = if matches!(active_pane, ActivePane::Logs) {
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Color::Cyan)
        };
        let logs_border_type = if matches!(active_pane, ActivePane::Logs) {
            BorderType::Double
        } else {
            BorderType::Plain
        };
        let logs_block = Block::bordered()
            .title(logs_title)
            .border_type(logs_border_type)
            .border_style(logs_border_style);

        let visible = logs_area.height.saturating_sub(2) as usize;
        let skip = log
            .len()
            .saturating_sub(visible + log_scroll as usize)
            .min(log.len());
        let lines: Vec<Line> = log
            .iter()
            .map(|msg| Line::styled(msg.clone(), log_style(msg)))
            .collect();
        let logs_paragraph = Paragraph::new(Text::from(lines))
            .block(logs_block)
            .scroll((skip as u16, 0));
        frame.render_widget(logs_paragraph, logs_area);
    } else {
        // Stacked layout for narrow terminals: Detected, Added, Logs
        let detected_height = height / 3;
        let added_height = height / 3;
        let logs_height = height - detected_height - added_height;

        let [detected_area, middle_area] =
            Layout::vertical([Constraint::Length(detected_height), Constraint::Fill(1)])
                .areas(area);

        let [added_area, logs_area] = Layout::vertical([
            Constraint::Length(added_height),
            Constraint::Length(logs_height),
        ])
        .areas(middle_area);

        // Render Detected pane
        let detected_theme = PanelTheme::new("Detected", Color::Yellow, "detected");
        let detected_border_style = if matches!(active_pane, ActivePane::Detected) {
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Color::Yellow)
        };
        let detected_border_type = if matches!(active_pane, ActivePane::Detected) {
            BorderType::Double
        } else {
            BorderType::Plain
        };

        let detected_rows = detected.iter().enumerate().map(|(i, d)| {
            let now = Instant::now();
            let pulse_duration = Duration::from_millis(PULSE_MS);
            let flash = flashes
                .get(&d.key)
                .filter(|f| now.duration_since(f.start) < super::logging::flash_duration(f.level));
            let pulse = pulses
                .get(&d.key)
                .map(|t| now.duration_since(*t))
                .filter(|e| *e < pulse_duration);
            let status = if let Some(f) = flash {
                f.msg.clone()
            } else if pulse.is_some() {
                "beacon received".to_string()
            } else {
                detected_theme.idle_status.to_string()
            };
            let style = row_style(
                detected_theme.border,
                detected_selected == Some(i),
                flash,
                pulse,
                now,
            );
            Row::new([
                Cell::from(d.name.clone()),
                Cell::from(d.ip.clone().unwrap_or_else(|| "-".to_string())),
                Cell::from(d.port.to_string()),
                Cell::from(status),
            ])
            .style(style)
        });
        let detected_table = Table::new(
            detected_rows,
            [
                Constraint::Percentage(35),
                Constraint::Percentage(30),
                Constraint::Length(5),
                Constraint::Min(8),
            ],
        )
        .header(Row::new(["Name", "IP", "Port", "Status"]).style(Style::new().bold()))
        .block(
            Block::bordered()
                .title(format!("{} — {}", detected_theme.title, detected.len()))
                .border_type(detected_border_type)
                .border_style(detected_border_style),
        );
        frame.render_widget(detected_table, detected_area);

        // Render Added pane
        let added_theme = PanelTheme::new("Added", Color::Green, "known");
        let added_border_style = if matches!(active_pane, ActivePane::Added) {
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Color::Green)
        };
        let added_border_type = if matches!(active_pane, ActivePane::Added) {
            BorderType::Double
        } else {
            BorderType::Plain
        };

        let added_rows = added.iter().enumerate().map(|(i, d)| {
            let now = Instant::now();
            let pulse_duration = Duration::from_millis(PULSE_MS);
            let flash = flashes
                .get(&d.key)
                .filter(|f| now.duration_since(f.start) < super::logging::flash_duration(f.level));
            let pulse = pulses
                .get(&d.key)
                .map(|t| now.duration_since(*t))
                .filter(|e| *e < pulse_duration);
            let status = if let Some(f) = flash {
                f.msg.clone()
            } else if pulse.is_some() {
                "beacon received".to_string()
            } else {
                added_theme.idle_status.to_string()
            };
            let style = row_style(
                added_theme.border,
                added_selected == Some(i),
                flash,
                pulse,
                now,
            );
            Row::new([
                Cell::from(d.name.clone()),
                Cell::from(d.ip.clone().unwrap_or_else(|| "-".to_string())),
                Cell::from(d.port.to_string()),
                Cell::from(status),
            ])
            .style(style)
        });
        let added_table = Table::new(
            added_rows,
            [
                Constraint::Percentage(35),
                Constraint::Percentage(30),
                Constraint::Length(5),
                Constraint::Min(8),
            ],
        )
        .header(Row::new(["Name", "IP", "Port", "Status"]).style(Style::new().bold()))
        .block(
            Block::bordered()
                .title(format!("{} — {}", added_theme.title, added.len()))
                .border_type(added_border_type)
                .border_style(added_border_style),
        );
        frame.render_widget(added_table, added_area);

        // Render Logs pane
        let logs_title = if verbose { "Logs — verbose" } else { "Logs" };
        let logs_border_style = if matches!(active_pane, ActivePane::Logs) {
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Color::Cyan)
        };
        let logs_border_type = if matches!(active_pane, ActivePane::Logs) {
            BorderType::Double
        } else {
            BorderType::Plain
        };
        let logs_block = Block::bordered()
            .title(logs_title)
            .border_type(logs_border_type)
            .border_style(logs_border_style);

        let visible = logs_area.height.saturating_sub(2) as usize;
        let skip = log
            .len()
            .saturating_sub(visible + log_scroll as usize)
            .min(log.len());
        let lines: Vec<Line> = log
            .iter()
            .map(|msg| Line::styled(msg.clone(), log_style(msg)))
            .collect();
        let logs_paragraph = Paragraph::new(Text::from(lines))
            .block(logs_block)
            .scroll((skip as u16, 0));
        frame.render_widget(logs_paragraph, logs_area);
    }
}

/// Question block, always the bottom block of the screen. Returns the text
/// cursor position when the user is typing a name. While the help panel is
/// shown (`help`), the block describes the help panel instead.
pub fn render_question(
    frame: &mut Frame,
    area: Rect,
    question: &super::types::Question,
    ctx: &QuestionCtx,
) -> Option<Position> {
    // The status hints mention the verbose key with its current state.
    let verbose_hint = format!("[v] verbose: {}", if ctx.verbose { "on" } else { "off" });
    let (title, border, question_line, hint_line) = if ctx.help
        && matches!(question, super::types::Question::Idle)
    {
        (
            "Help",
            Color::Cyan,
            "Command-line and interactive help".to_string(),
            hint_spans(
                "[Up/Dn/PgUp/PgDn] scroll help   [?] close   [s] sync git   [q] quit",
                ctx.key_flash,
            ),
        )
    } else {
        match question {
            super::types::Question::Idle => {
                use super::types::ActivePane;
                // Hint lines depend on the active pane
                let (status_line, hint) = match ctx.active_pane {
                    ActivePane::MainMenu => {
                        (
                            "Main menu".to_string(),
                            "[d] Detected   [a] Added   [l] Logs   [?] help   [q] quit".to_string(),
                        )
                    }
                    ActivePane::Detected => {
                        if ctx.pending > 0 {
                            (
                                "Detected devices".to_string(),
                                format!("[Esc] no pane   [Up/Dn] select   [Enter/a] add   [n] ignore   [?] help   [q] quit"),
                            )
                        } else {
                            (
                                "Detected devices".to_string(),
                                "[Esc] no pane   [?] help   [q] quit".to_string(),
                            )
                        }
                    }
                    ActivePane::Added => {
                        (
                            "Added devices".to_string(),
                            "[Esc] no pane   [Up/Dn] select   [i] install key   [r] remove key   [d] delete   [s] sync   [w] wipe   [c] setup-cd   [?] help   [q] quit".to_string(),
                        )
                    }
                    ActivePane::Logs => {
                        (
                            if ctx.verbose { "Logs - verbose".to_string() } else { "Logs".to_string() },
                            "[Esc] no pane   [Up/Dn] scroll line   [PgUp/PgDn] scroll page   [v] toggle verbose   [?] help   [q] quit".to_string(),
                        )
                    }
                };
                (
                    "Actions",
                    Color::Blue,
                    status_line,
                    hint_spans(&hint, ctx.key_flash),
                )
            }
            super::types::Question::AddDevice { name, ip } => (
                "New device",
                Color::Yellow,
                format!("Add {name} ({ip}) to config?"),
                hint_spans(
                    &format!("[y] add   [n] skip   [s] sync git   {verbose_hint}   [q] quit"),
                    ctx.key_flash,
                ),
            ),
            super::types::Question::TrustKey { name } => (
                "Signing key changed",
                Color::Red,
                format!(
                    "Trust the new signing key for {name}? (expected after a reinstall — otherwise it may be spoofing)"
                ),
                hint_spans("[y] trust   [n] reject", ctx.key_flash),
            ),
            super::types::Question::Rename { name, suggested } => (
                "Name already used",
                Color::Cyan,
                format!("New name for {name} ([Enter] = {suggested}, [Esc] = skip)"),
                Line::from(format!("> {}", ctx.input)),
            ),
            super::types::Question::Delete { name } => (
                "Delete device",
                Color::Red,
                format!("Delete {name} from configuration?"),
                hint_spans(
                    "[Up/Dn] select   [Enter] confirm   [Esc] cancel",
                    ctx.key_flash,
                ),
            ),
            super::types::Question::Wipe { count } => (
                "Wipe configuration",
                Color::Red,
                format!(
                    "Delete the ENTIRE configuration ({count} device block(s) + ~/.regroup-config)?"
                ),
                hint_spans("[Enter/y] confirm   [Esc/w/q] cancel", ctx.key_flash),
            ),
            super::types::Question::SetupCd { file } => (
                "Install regroup-cd",
                Color::Yellow,
                format!(
                    "Install the 'regroup-cd' shell function (cd to ~/.regroup-config) in ~/{file}?"
                ),
                hint_spans("[Enter/y] confirm   [Esc/c/q] cancel", ctx.key_flash),
            ),
            super::types::Question::InstallKeySelectMachine { machines } => (
                "Install SSH key",
                Color::Green,
                format!("Select target machine ({} available)", machines.len()),
                hint_spans(
                    "[Up/Dn] select   [Enter] confirm   [Esc/q] cancel",
                    ctx.key_flash,
                ),
            ),
            super::types::Question::InstallKeySelectUser { machine, users } => (
                "Install SSH key",
                Color::Green,
                format!("Select user on {machine} ({} available)", users.len()),
                hint_spans(
                    "[Up/Dn] select   [Enter] confirm   [Esc] back",
                    ctx.key_flash,
                ),
            ),
            super::types::Question::InstallKeyConfirm {
                from_user,
                from_machine,
                to_user,
                to_machine,
            } => (
                "Install SSH key",
                Color::Green,
                format!(
                    "Install key from {}@{} to {}@{}?",
                    from_user, from_machine, to_user, to_machine
                ),
                hint_spans("[y] confirm   [n/Esc] cancel", ctx.key_flash),
            ),
            super::types::Question::RemoveKeySelectConnection { connections } => (
                "Remove SSH key",
                Color::Red,
                format!(
                    "Select connection to remove ({} available)",
                    connections.len()
                ),
                hint_spans(
                    "[Up/Dn] select   [Enter] confirm   [Esc/q] cancel",
                    ctx.key_flash,
                ),
            ),
            super::types::Question::RemoveKeyConfirm {
                connection_id,
                from_user,
                from_machine,
                to_user,
                to_machine,
            } => (
                "Remove SSH key",
                Color::Red,
                format!(
                    "Remove key {} ({}@{}) from {}@{}?",
                    connection_id, from_user, from_machine, to_user, to_machine
                ),
                hint_spans("[y] confirm   [n/Esc] cancel", ctx.key_flash),
            ),
        }
    };
    let text = Text::from(vec![
        Line::styled(question_line, Style::new().bold()),
        hint_line,
    ]);
    frame.render_widget(
        Paragraph::new(text).block(
            Block::bordered()
                .title(title)
                .border_style(Style::new().fg(border)),
        ),
        area,
    );
    if matches!(question, super::types::Question::Rename { .. }) {
        // Second line inside the block: "> " prefix plus the typed name.
        let x = area.x + 1 + 2 + ctx.input.chars().count() as u16;
        let y = area.y + 2;
        Some(Position::new(x, y))
    } else {
        None
    }
}
