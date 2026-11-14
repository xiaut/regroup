//! UI rendering functions and styling utilities.
//!
//! Contains all the rendering logic for the TUI interface, including device
//! panels, activity log, help panel, and question block rendering.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};

use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Cell, Paragraph, Row, Table, Wrap};

use super::keys::{self, Context};
use super::logging::{BLINK_MS, KEY_FLASH_MS, PULSE_MS};
use super::types::{bindings_hint_text, ActivePane, Level, QuestionDef, HELP_BINDINGS};

/// Hint text for the Idle action bar, built from the pane's key list —
/// the same list key presses are dispatched through.
pub fn idle_hint_text(ctx: Context) -> String {
    keys::hints(ctx)
        .iter()
        .map(|h| format!("[{}] {}", h.keys, h.description))
        .collect::<Vec<_>>()
        .join("   ")
}

/// Hint text for a modal question: the question's own bindings, exactly
/// what key presses are resolved against.
pub fn modal_hint_text(def: &QuestionDef) -> String {
    bindings_hint_text(def.bindings)
}

/// Hint text while the help panel is open.
pub fn help_hint_text() -> String {
    bindings_hint_text(HELP_BINDINGS)
}

/// Minimum terminal width to display device panels side-by-side.
/// Below this, panels stack vertically.
pub const MIN_WIDTH_FOR_SIDE_BY_SIDE: u16 = 80;

/// Wrap `text` to `width` the way the question block's `Paragraph`
/// (`Wrap { trim: false }`) does, and return `(line_count, last_line_width)`.
/// Hint-like text (with "   " separators) is wrapped token by token so a
/// `[key] desc` pair is never split; regular text wraps on words.
pub(crate) fn wrapped_metrics(text: &str, width: usize, is_hint: bool) -> (usize, usize) {
    if width == 0 {
        let lines = text.lines().count().max(1);
        let last = text.lines().last().map_or(0, |l| l.chars().count());
        return (lines, last);
    }

    // For hint text, treat "   " separated tokens as non-breakable
    // For regular text (question lines), use word-based wrapping
    let tokens: Vec<&str> = if is_hint && text.contains("   ") {
        text.split("   ").collect()
    } else {
        text.split_whitespace().collect()
    };

    let separator_len = if is_hint && text.contains("   ") { 3 } else { 1 };

    let mut lines = 0;
    let mut current_width = 0;

    for token in tokens {
        let token_len = token.chars().count();

        if current_width == 0 {
            // First token on this line
            current_width = token_len;
        } else if current_width + separator_len + token_len <= width {
            // Token fits on current line
            current_width += separator_len + token_len;
        } else {
            // Token doesn't fit, move to next line
            lines += 1;
            current_width = token_len;
        }
    }

    // Don't forget the last line
    if current_width > 0 {
        lines += 1;
    }

    (lines.max(1), current_width)
}

/// How many lines `text` occupies when wrapped to `width` in the question
/// block; see [`wrapped_metrics`].
pub(crate) fn count_wrapped_lines(text: &str, width: usize, is_hint: bool) -> usize {
    wrapped_metrics(text, width, is_hint).0
}

/// Static look of one device panel: title, border color and the status
/// shown when the device has no animated message.
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

/// A device's animated status, shown in the last column of its row.
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
    /// Devices waiting for a decision (drives the Idle hints).
    pub pending: usize,
    /// Whether the help panel is shown.
    pub help: bool,
    /// Which pane is currently active.
    pub active_pane: super::types::ActivePane,
    /// Current navigation state (focus + depth), drives the hint line.
    pub navigation: super::types::Navigation,
    /// Whether the selected user is confirmed (drives Users-pane hints).
    pub confirmed_user: bool,
    /// Last key press, highlighted in the hint line while recent.
    pub key_flash: Option<&'a KeyFlash>,
}

/// A device panel: one row per device, with the animated per-device
/// status in the last column. `selected` highlights a row (selection in
/// the Detected panel, delete mode in the Added panel). A recent beacon
/// pulses the row: bright background fading back to the base color.
pub fn render_devices(
    frame: &mut Frame,
    area: Rect,
    theme: PanelTheme,
    devices: &[super::types::UiDevice],
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

/// Style of a device row: the panel's base color, overridden by the
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
        KeyCode::Down => Some("Down".to_string()),
        KeyCode::Left => Some("Left".to_string()),
        KeyCode::Right => Some("Right".to_string()),
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

/// Logs pane: bottom-anchored log, shifted up by `scroll` lines.
pub fn render_log(frame: &mut Frame, area: Rect, log: &[String], scroll: u16) {
    let visible = area.height.saturating_sub(2) as usize;
    let skip = log
        .len()
        .saturating_sub(visible + scroll as usize)
        .min(log.len());
    let lines: Vec<Line> = log
        .iter()
        .map(|msg| Line::styled(msg.clone(), log_style(msg)))
        .collect();
    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::bordered().title("Logs"))
        .scroll((skip as u16, 0));
    frame.render_widget(paragraph, area);
}

/// Help panel (key '?'): the CLI help text, replacing the device panels
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
    } else if msg.starts_with("WARNING") {
        Style::new().fg(Color::Yellow)
    } else if msg.starts_with("[~]") {
        Style::new().fg(Color::Cyan)
    } else if msg.starts_with("[DRY RUN]") {
        Style::new().fg(Color::Magenta)
    } else {
        Style::new()
    }
}

/// Users sub-panel of the Added pane: the users of the selected device,
/// one row per user with the status column (temporary / confirmed / SSH key
/// installed). Highlighted border when the panel is the active one.
pub fn render_users_panel(
    frame: &mut Frame,
    area: Rect,
    users: &[super::types::UiUser],
    selected: Option<usize>,
    active: bool,
) {
    use super::types::UserStatus;
    let border = if active { Color::Magenta } else { Color::DarkGray };
    let border_type = if active { BorderType::Double } else { BorderType::Plain };
    let rows = users.iter().enumerate().map(|(i, u)| {
        let mut style = match u.status {
            UserStatus::Temporary => Style::new().fg(Color::Yellow),
            UserStatus::Confirmed => Style::new().fg(Color::Green),
            UserStatus::KeyInstalled => Style::new().fg(Color::Cyan),
        };
        if selected == Some(i) {
            style = Style::new().add_modifier(Modifier::REVERSED);
        }
        Row::new([Cell::from(u.name.clone()), Cell::from(u.status_label())]).style(style)
    });
    let table = Table::new(
        rows,
        [Constraint::Percentage(45), Constraint::Percentage(55)],
    )
    .header(Row::new(["User", "Status"]).style(Style::new().bold()))
    .block(
        Block::bordered()
            .title(format!("Users — {}", users.len()))
            .border_type(border_type)
            .border_style(Style::new().fg(border)),
    );
    frame.render_widget(table, area);
}

/// Render panes based on the active pane.
/// When a pane is active, it takes the full screen (full-screen mode).
/// When several panels are visible at once (the Added pane and its Users
/// sub-panel), the active one is marked with a double border; a lone
/// full-screen panel keeps plain borders.
/// When in MainMenu, show all three panes side-by-side or stacked.
pub fn render_all_panes(
    frame: &mut Frame,
    area: Rect,
    active_pane: ActivePane,
    navigation_focus: super::types::NavFocus,
    detected: &[super::types::UiDevice],
    detected_selected: Option<usize>,
    added: &[super::types::UiDevice],
    added_selected: Option<usize>,
    log: &[String],
    log_scroll: u16,
    flashes: &HashMap<String, RowFlash>,
    pulses: &HashMap<String, Instant>,
    users: &[super::types::UiUser],
    users_selected: Option<usize>,
) {
    let width = area.width;
    let height = area.height;

    // Minimum width for side-by-side top panes
    let min_width_for_side_by_side = 80;

    // If a specific pane is active (not MainMenu), show only that pane full-screen
    match active_pane {
        ActivePane::Detected(_) => {
            // Full-screen Detected pane
            let detected_theme = PanelTheme::new("Detected", Color::Yellow, "detected");
            let detected_border_style = Style::new().fg(Color::Yellow);
            
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
                    .title(format!("Detected — {}", detected.len()))
                    .border_type(BorderType::Plain)
                    .border_style(detected_border_style),
            );
            frame.render_widget(detected_table, area);
        }
        ActivePane::Added(_) | ActivePane::Users(_) => {
            // Full-screen devices pane with the Users sub-panel on the
            // right: the users of the device selected in the Added panel.
            let devices_width = (width * 3) / 5;
            let [devices_area, users_area] = Layout::horizontal([
                Constraint::Length(devices_width),
                Constraint::Fill(1),
            ])
            .areas(area);

            let added_theme = PanelTheme::new("Added", Color::Green, "known");
            // A deactivated panel (another panel is active) gets gray
            // borders; the active one keeps its theme color.
            let added_active = matches!(active_pane, ActivePane::Added(_));
            let added_border_style = if added_active {
                Style::new().fg(Color::Green)
            } else {
                Style::new().fg(Color::DarkGray)
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
                    .title(format!("Added — {}", added.len()))
                    .border_type(if added_active {
                        BorderType::Double
                    } else {
                        BorderType::Plain
                    })
                    .border_style(added_border_style),
            );
            frame.render_widget(added_table, devices_area);

            render_users_panel(
                frame,
                users_area,
                users,
                users_selected,
                matches!(active_pane, ActivePane::Users(_)),
            );
        }
        ActivePane::Logs(_) => {
            // Full-screen Logs pane
            let logs_title = "Logs";
            let logs_border_style = Style::new().fg(Color::Cyan);
            let logs_block = Block::bordered()
                .title(logs_title)
                .border_type(BorderType::Plain)
                .border_style(logs_border_style);

            let visible = height.saturating_sub(2) as usize;
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
            frame.render_widget(logs_paragraph, area);
        }
        ActivePane::MainMenu(_) => {
            // Show all three panes when in MainMenu
            // Highlight the panel that has focus
            let detected_selected_focus = matches!(navigation_focus, super::types::NavFocus::Detected);
            let added_selected_focus = matches!(navigation_focus, super::types::NavFocus::Added);
            let logs_selected_focus = matches!(navigation_focus, super::types::NavFocus::Logs);
            
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
                let detected_border_style = if detected_selected_focus {
                    Style::new().fg(Color::Yellow).bold()
                } else {
                    Style::new().fg(Color::Yellow)
                };
                let detected_border_type = if detected_selected_focus {
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
                let added_border_style = if added_selected_focus {
                    Style::new().fg(Color::Green).bold()
                } else {
                    Style::new().fg(Color::Green)
                };
                let added_border_type = if added_selected_focus {
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
                let logs_title = "Logs";
                let logs_border_type = if logs_selected_focus {
                    BorderType::Double
                } else {
                    BorderType::Plain
                };
                let logs_border_style = if logs_selected_focus {
                    Style::new().fg(Color::Cyan).bold()
                } else {
                    Style::new().fg(Color::Cyan)
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
                let detected_border_style = if detected_selected_focus {
                    Style::new().fg(Color::Yellow).bold()
                } else {
                    Style::new().fg(Color::Yellow)
                };
                let detected_border_type = if detected_selected_focus {
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
                let added_border_style = if added_selected_focus {
                    Style::new().fg(Color::Green).bold()
                } else {
                    Style::new().fg(Color::Green)
                };
                let added_border_type = if added_selected_focus {
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
                let logs_title = "Logs";
                let logs_border_type = if logs_selected_focus {
                    BorderType::Double
                } else {
                    BorderType::Plain
                };
                let logs_border_style = if logs_selected_focus {
                    Style::new().fg(Color::Cyan).bold()
                } else {
                    Style::new().fg(Color::Cyan)
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
    let (title, border, question_line, hint_line) = if ctx.help
        && matches!(question, super::types::Question::Idle(_))
    {
        (
            "Help",
            Color::Cyan,
            "Command-line and interactive help".to_string(),
            hint_spans(&help_hint_text(), ctx.key_flash),
        )
    } else {
        match question {
            super::types::Question::Idle(def) => {
                // Hint line driven by the navigation key table: shows the
                // keys that are actually handled in the current focus + depth.
                let status_line = ctx.active_pane.name().to_string();
                let hint = idle_hint_text(Context::Pane {
                    focus: ctx.navigation.focus,
                    depth: ctx.navigation.depth,
                    confirmed_user: ctx.confirmed_user,
                });
                (
                    def.title,
                    Color::Blue,
                    status_line,
                    hint_spans(&hint, ctx.key_flash),
                )
            }
            super::types::Question::AddDevice(def, name, ip) => (
                def.title,
                Color::Yellow,
                format!("Add {name} ({ip}) to config?"),
                hint_spans(&modal_hint_text(def), ctx.key_flash),
            ),
            super::types::Question::TrustKey(def, name) => (
                def.title,
                Color::Red,
                format!(
                    "Trust the new signing key for {name}? (expected after a reinstall — otherwise it may be spoofing)"
                ),
                hint_spans(&modal_hint_text(def), ctx.key_flash),
            ),
            super::types::Question::Rename(def, name, suggested) => (
                def.title,
                Color::Cyan,
                format!(
                    "New name for {name} ({}; empty = {suggested})",
                    bindings_hint_text(def.bindings)
                ),
                Line::from(format!("> {}", ctx.input)),
            ),
            super::types::Question::AddTempUser(def, device) => (
                def.title,
                Color::Cyan,
                format!(
                    "Temporary user on {device} (kept in tmp/) — {}",
                    bindings_hint_text(def.bindings)
                ),
                Line::from(format!("> {}", ctx.input)),
            ),
            super::types::Question::Delete(def, name) => (
                def.title,
                Color::Red,
                format!("Delete {name} from configuration?"),
                hint_spans(&modal_hint_text(def), ctx.key_flash),
            ),
            super::types::Question::Wipe(def, count) => (
                def.title,
                Color::Red,
                format!(
                    "Delete the ENTIRE configuration ({count} device block(s) + ~/.regroup-config)?"
                ),
                hint_spans(&modal_hint_text(def), ctx.key_flash),
            ),
            super::types::Question::SetupCd(def, file) => (
                def.title,
                Color::Yellow,
                format!(
                    "Install the 'regroup-cd' shell function (cd to ~/.regroup-config) in ~/{file}?"
                ),
                hint_spans(&modal_hint_text(def), ctx.key_flash),
            ),
        }
    };
    // Wrapped height of the question line, needed for the text cursor below.
    let question_lines = count_wrapped_lines(
        &question_line,
        area.width.saturating_sub(2) as usize,
        false,
    );
    let text = Text::from(vec![
        Line::styled(question_line, Style::new().bold()),
        hint_line,
    ]);
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(
                Block::bordered()
                    .title(title)
                    .border_style(Style::new().fg(border)),
            ),
        area,
    );
    if matches!(
        question,
        super::types::Question::Rename { .. } | super::types::Question::AddTempUser { .. }
    ) {
        // The input line sits under the (possibly wrapped) question line:
        // "> " prefix plus the typed text, itself wrapped.
        let content_width = area.width.saturating_sub(2) as usize;
        let input_chars = ctx.input.chars().count();
        let (input_lines, last_len) = if content_width == 0 || 2 + input_chars <= content_width {
            (1, 2 + input_chars)
        } else {
            wrapped_metrics(&format!("> {}", ctx.input), content_width, false)
        };
        let x = (area.x + 1 + last_len as u16).min(area.right().saturating_sub(1));
        let y = (area.y + 1 + question_lines as u16 + input_lines.saturating_sub(1) as u16)
            .min(area.bottom().saturating_sub(1));
        Some(Position::new(x, y))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::types::{NavFocus, Navigation, Question};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn ctx(input: &str) -> QuestionCtx<'_> {
        QuestionCtx {
            input,
            pending: 0,
            help: false,
            active_pane: ActivePane::main_menu(),
            navigation: Navigation {
                focus: NavFocus::Center,
                depth: None,
            },
            confirmed_user: false,
            key_flash: None,
        }
    }

    fn question_line_of(q: &Question) -> String {
        match q {
            Question::Rename(def, name, suggested) => format!(
                "New name for {name} ({}; empty = {suggested})",
                bindings_hint_text(def.bindings)
            ),
            _ => unreachable!(),
        }
    }

    fn draw_cursor(terminal_width: u16, area: Rect, q: &Question, input: &str) -> Option<Position> {
        let backend = TestBackend::new(terminal_width, area.y + area.height);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut pos = None;
        terminal
            .draw(|frame| {
                pos = render_question(frame, area, q, &ctx(input));
            })
            .unwrap();
        pos
    }

    #[test]
    fn text_cursor_sits_after_the_input_on_a_single_line_question() {
        let q = Question::rename("dev".to_string(), "dev2".to_string());
        let area = Rect::new(0, 6, 120, 4);
        let pos = draw_cursor(120, area, &q, "abc").unwrap();
        assert_eq!(pos, Position::new(area.x + 1 + 2 + 3, area.y + 2));
    }

    #[test]
    fn text_cursor_follows_the_wrapped_question_line() {
        let name = "a-very-long-device-hostname-that-wraps".to_string();
        let q = Question::rename(name, "suggested".to_string());
        let area = Rect::new(0, 6, 40, 8);
        let content_width = (area.width - 2) as usize;
        let question_lines = count_wrapped_lines(&question_line_of(&q), content_width, false);
        assert!(question_lines > 1, "test needs a wrapping question line");
        let pos = draw_cursor(40, area, &q, "abc").unwrap();
        assert_eq!(
            pos,
            Position::new(area.x + 1 + 2 + 3, area.y + 1 + question_lines as u16)
        );
    }
}
