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
use super::types::{
    bindings_hint_text, ActivePane, DeviceSection, Level, PaneKeyCtx, QuestionDef, UiDevice,
    HELP_BINDINGS,
};

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

/// Height of the Logs strip (borders included) kept visible under a
/// fullscreen device pane, like the action pane below the main area.
pub const LOG_STRIP_HEIGHT: u16 = 6;

/// Minimum height of the main area for the Logs strip to appear under a
/// fullscreen pane; below this the pane keeps the whole area.
pub const MIN_HEIGHT_FOR_LOG_STRIP: u16 = 12;

/// Split `area` into the pane area and the always-visible Logs strip:
/// `(pane_area, Some(logs_area))` when the strip fits, `(area, None)`
/// when the main area is too short.
pub fn split_log_strip(area: Rect) -> (Rect, Option<Rect>) {
    if area.height >= MIN_HEIGHT_FOR_LOG_STRIP {
        let [pane_area, logs_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(LOG_STRIP_HEIGHT)])
                .areas(area);
        (pane_area, Some(logs_area))
    } else {
        (area, None)
    }
}

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

/// Divider row between two groups of a pane (a Devices-pane section, a
/// Users-pane membership group): a non-selectable labeled line in dark
/// gray, padded to the pane's `columns`.
fn divider_row(label: &str, columns: usize) -> Row<'static> {
    let mut cells = vec![Cell::from(format!("── {label} ──"))];
    cells.resize_with(columns, || Cell::from(""));
    Row::new(cells).style(Style::new().fg(Color::DarkGray))
}

/// The rows of the Devices pane: one row per device, each section
/// opening with its divider row (localhost, then added, then
/// detected). `selected` is an index into `devices`; dividers are
/// not selectable.
fn device_rows(
    devices: &[UiDevice],
    selected: Option<usize>,
    flashes: &HashMap<String, RowFlash>,
    pulses: &HashMap<String, Instant>,
    base: Color,
) -> Vec<Row<'static>> {
    let now = Instant::now();
    let pulse_duration = Duration::from_millis(PULSE_MS);
    let mut rows: Vec<Row<'static>> = Vec::new();
    let mut last_section: Option<DeviceSection> = None;
    for (i, d) in devices.iter().enumerate() {
        // Every section opens with its divider row, in the same style:
        // "── added ──", "── detected ──" — and the localhost divider
        // carries the super-state of the local device:
        // "── localhost (added) ──" / "── localhost (not added) ──"
        // (a section is non-empty here: we are about to render one of
        // its rows).
        if last_section != Some(d.section) {
            let label = if d.section == DeviceSection::Local {
                format!(
                    "localhost ({})",
                    if d.in_shared_config { "added" } else { "not added" }
                )
            } else {
                d.section.label().to_string()
            };
            rows.push(divider_row(&label, 4));
        }
        last_section = Some(d.section);
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
            d.section.idle_status().to_string()
        };
        let style = row_style(base, selected == Some(i), flash, pulse, now);
        rows.push(
            Row::new([
                Cell::from(d.name.clone()),
                Cell::from(d.ip.clone().unwrap_or_else(|| "-".to_string())),
                Cell::from(d.port.to_string()),
                Cell::from(status),
            ])
            .style(style),
        );
    }
    rows
}

/// The Devices pane: one table where each section opens with its
/// divider row — localhost, added, detected.
fn render_devices_pane(
    frame: &mut Frame,
    area: Rect,
    devices: &[UiDevice],
    selected: Option<usize>,
    flashes: &HashMap<String, RowFlash>,
    pulses: &HashMap<String, Instant>,
    border_style: Style,
    border_type: BorderType,
) {
    let added = devices
        .iter()
        .filter(|d| d.section == DeviceSection::Added)
        .count();
    let detected = devices
        .iter()
        .filter(|d| d.section == DeviceSection::Detected)
        .count();
    let rows = device_rows(devices, selected, flashes, pulses, Color::Green);
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
            .title(format!("Devices — {added} added, {detected} detected"))
            .border_type(border_type)
            .border_style(border_style),
    );
    frame.render_widget(table, area);
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
    /// Whether the selected Users row is an external row (an incoming
    /// connection of the localhost device) — drives the revoke key.
    pub selected_user_external: bool,
    /// Section of the selected row in the Devices pane (drives the
    /// Devices-pane hints: which action keys are offered).
    pub selected_section: Option<DeviceSection>,
    /// Whether the local device is in the shared configuration (drives
    /// the localhost row's add/remove key).
    pub local_in_config: bool,
    /// Whether the device selected in the Devices pane has at least one
    /// user (drives the Right/Enter entry keys into the Users pane).
    pub has_users: bool,
    /// Last key press, highlighted in the hint line while recent.
    pub key_flash: Option<&'a KeyFlash>,
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

/// Lines to show in a log pane of `visible` content rows: the tail of
/// `log`, shifted up by `scroll` lines. When the remaining lines are
/// fewer than the pane, blank lines are prepended so the log is
/// bottom-anchored and the newest line always sits on the pane's
/// last row.
pub(crate) fn log_lines(log: &[String], visible: usize, scroll: u16) -> Vec<Line<'static>> {
    let skip = log
        .len()
        .saturating_sub(visible + scroll as usize)
        .min(log.len());
    let shown = (log.len() - skip).min(visible);
    let pad = visible - shown;
    let mut lines = Vec::with_capacity(visible);
    lines.extend(std::iter::repeat_n(Line::default(), pad));
    lines.extend(
        log.iter()
            .skip(skip)
            .take(shown)
            .map(|msg| Line::styled(msg.clone(), log_style(msg))),
    );
    lines
}

/// Logs pane: the tail of the log, bottom-anchored, shifted up by
/// `scroll` lines; see [`log_lines`].
pub fn render_log(
    frame: &mut Frame,
    area: Rect,
    log: &[String],
    scroll: u16,
    border_style: Style,
    border_type: BorderType,
) {
    let visible = area.height.saturating_sub(2) as usize;
    let paragraph = Paragraph::new(Text::from(log_lines(log, visible, scroll))).block(
        Block::bordered()
            .title("Logs")
            .border_style(border_style)
            .border_type(border_type),
    );
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

/// Users sub-panel of the Devices pane: the users of the selected device
/// (local or added), grouped with divider rows: the device's default user
/// first under `── default ──` (at most one per device), then the pinned
/// row on the localhost device (`── current user (added) ──` /
/// `── current user (not added) ──`), then `── added ──` (users in the
/// shared configuration), then `── not added ──`. Each user row keeps the
/// sub-status column (temporary / confirmed / SSH key installed).
/// `selected` is an index into `users`; dividers are not selectable.
/// Highlighted border when the panel is the active one.
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
    // The super-state of each user, in display order: the default user
    // first, then the pinned row, then added, then not added (users_view
    // returns them in this order).
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Group {
        Default,
        Pinned,
        Added,
        NotAdded,
        External,
    }
    let group_of = |u: &super::types::UiUser| {
        if u.incoming.is_some() {
            Group::External
        } else if u.is_default {
            Group::Default
        } else if u.pinned {
            Group::Pinned
        } else if u.added {
            Group::Added
        } else {
            Group::NotAdded
        }
    };
    let mut rows: Vec<Row<'static>> = Vec::new();
    let mut last_group: Option<Group> = None;
    for (i, u) in users.iter().enumerate() {
        let group = group_of(u);
        if last_group != Some(group) {
            let label = match group {
                Group::Default => "default".to_string(),
                Group::Pinned => format!(
                    "current user ({})",
                    if u.added { "added" } else { "not added" }
                ),
                Group::Added => "added".to_string(),
                Group::NotAdded => "not added".to_string(),
                // Incoming connections of this device (localhost only),
                // targeting the current user — see users_view.
                Group::External => "external".to_string(),
            };
            rows.push(divider_row(&label, 2));
        }
        last_group = Some(group);
        let mut style = match u.status {
            UserStatus::Temporary => Style::new().fg(Color::Yellow),
            UserStatus::Confirmed => Style::new().fg(Color::Green),
            UserStatus::KeyInstalled => Style::new().fg(Color::Cyan),
            UserStatus::IncomingKey => Style::new().fg(Color::Magenta),
        };
        if selected == Some(i) {
            style = Style::new().add_modifier(Modifier::REVERSED);
        }
        rows.push(
            Row::new([Cell::from(u.name.clone()), Cell::from(u.status_label())]).style(style),
        );
    }
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
/// When a pane is active, it fills the area it is given (full-screen
/// mode; the caller keeps the Logs strip visible below it).
/// When several panels are visible at once (the Devices pane and its
/// Users sub-panel), the active one is marked with a double border; a
/// lone full-screen panel keeps plain borders.
/// When in MainMenu, the dashboard shows the Devices pane on top (full
/// width) and the Logs pane below.
pub fn render_all_panes(
    frame: &mut Frame,
    area: Rect,
    active_pane: ActivePane,
    navigation_focus: super::types::NavFocus,
    devices: &[UiDevice],
    devices_selected: Option<usize>,
    log: &[String],
    log_scroll: u16,
    flashes: &HashMap<String, RowFlash>,
    pulses: &HashMap<String, Instant>,
    users: &[super::types::UiUser],
    users_selected: Option<usize>,
) {
    let width = area.width;
    let height = area.height;

    // If a specific pane is active (not MainMenu), show only that pane full-screen
    match active_pane {
        ActivePane::Devices(_) | ActivePane::Users(_) => {
            // Full-screen Devices pane with the Users sub-panel on the
            // right: the users of the device selected in the pane (local
            // or added).
            let devices_width = (width * 3) / 5;
            let [devices_area, users_area] = Layout::horizontal([
                Constraint::Length(devices_width),
                Constraint::Fill(1),
            ])
            .areas(area);

            // A deactivated panel (another panel is active) gets gray
            // borders; the active one keeps its theme color.
            let devices_active = matches!(active_pane, ActivePane::Devices(_));
            let devices_border_style = if devices_active {
                Style::new().fg(Color::Green)
            } else {
                Style::new().fg(Color::DarkGray)
            };
            let devices_border_type = if devices_active {
                BorderType::Double
            } else {
                BorderType::Plain
            };
            render_devices_pane(
                frame,
                devices_area,
                devices,
                devices_selected,
                flashes,
                pulses,
                devices_border_style,
                devices_border_type,
            );

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
            let logs_border_style = Style::new().fg(Color::Cyan);
            let logs_block = Block::bordered()
                .title("Logs")
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
            // Overview: the Devices pane fills the whole area (the Logs
            // strip below it is drawn by the caller). The focused panel
            // gets a double, bold border.
            let devices_focus = matches!(navigation_focus, super::types::NavFocus::Devices);
            let devices_border_style = if devices_focus {
                Style::new().fg(Color::Green).bold()
            } else {
                Style::new().fg(Color::Green)
            };
            let devices_border_type = if devices_focus {
                BorderType::Double
            } else {
                BorderType::Plain
            };
            render_devices_pane(
                frame,
                area,
                devices,
                devices_selected,
                flashes,
                pulses,
                devices_border_style,
                devices_border_type,
            );
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
                    keys_ctx: PaneKeyCtx {
                        selected_section: ctx.selected_section,
                        local_in_config: ctx.local_in_config,
                        user_external: ctx.selected_user_external,
                        has_users: ctx.has_users,
                    },
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
                    "Add user on {device} (kept in tmp/ until a sync or a key install confirms it) — {}",
                    bindings_hint_text(def.bindings)
                ),
                Line::from(format!("> {}", ctx.input)),
            ),
            super::types::Question::InstallKey(def, device, user) => (
                def.title,
                Color::Yellow,
                format!("Install an SSH key to {user}@{device}? (a device-scoped git sync follows)"),
                hint_spans(&modal_hint_text(def), ctx.key_flash),
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
            selected_user_external: false,
            selected_section: None,
            local_in_config: false,
            has_users: false,
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

    #[test]
    fn log_strip_sits_at_the_bottom_of_the_main_area() {
        let area = Rect::new(0, 0, 80, 30);
        let (pane_area, logs_area) = split_log_strip(area);
        let logs_area = logs_area.expect("strip should show at height 30");
        assert_eq!(logs_area.height, LOG_STRIP_HEIGHT);
        assert_eq!(pane_area.height, area.height - LOG_STRIP_HEIGHT);
        assert_eq!(logs_area.y, pane_area.y + pane_area.height);
        assert_eq!(logs_area.width, area.width);
    }

    #[test]
    fn log_strip_is_dropped_when_the_main_area_is_too_short() {
        let area = Rect::new(0, 0, 80, MIN_HEIGHT_FOR_LOG_STRIP - 1);
        let (pane_area, logs_area) = split_log_strip(area);
        assert!(logs_area.is_none());
        assert_eq!(pane_area, area);
    }

    #[test]
    fn fullscreen_pane_and_log_strip_render_side_by_side_without_overlap() {
        let area = Rect::new(0, 0, 80, 24);
        let (pane_area, logs_area) = split_log_strip(area);
        let logs_area = logs_area.unwrap();
        let log = vec!["hello".to_string(), "SECURITY something".to_string()];
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_all_panes(
                    frame,
                    pane_area,
                    ActivePane::devices(),
                    NavFocus::Devices,
                    &[],
                    None,
                    &log,
                    0,
                    &HashMap::new(),
                    &HashMap::new(),
                    &[],
                    None,
                );
                render_log(frame, logs_area, &log, 0, Style::new().fg(Color::Cyan), BorderType::Plain);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // The pane's bottom border sits right above the strip's top border.
        // The Devices pane is active here (the Users panel is visible next
        // to it), so it is drawn with a double border.
        assert_eq!(buf.cell((0, pane_area.bottom() - 1)).unwrap().symbol(), "╚");
        assert_eq!(buf.cell((0, logs_area.y)).unwrap().symbol(), "┌");
        // The strip is titled "Logs".
        assert_eq!(buf.cell((1, logs_area.y)).unwrap().symbol(), "L");
        // A log line is visible inside the strip.
        assert!(
            buf.content()
                .iter()
                .any(|c| c.symbol() == "S" && c.fg == Color::Red),
            "the SECURITY log line should be rendered in red"
        );
    }

    fn line_text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn log_of(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line{i}")).collect()
    }

    #[test]
    fn log_lines_shows_the_tail_when_the_log_is_longer_than_the_pane() {
        let texts: Vec<String> = log_lines(&log_of(10), 4, 0).iter().map(line_text).collect();
        assert_eq!(texts, ["line6", "line7", "line8", "line9"]);
    }

    #[test]
    fn log_lines_is_bottom_anchored_when_the_log_is_shorter_than_the_pane() {
        let texts: Vec<String> = log_lines(&log_of(2), 4, 0).iter().map(line_text).collect();
        // Blank-padded at the top so the newest line is on the last row.
        assert_eq!(texts, ["", "", "line0", "line1"]);
    }

    #[test]
    fn log_lines_scroll_shifts_the_window_up_from_the_tail() {
        let texts: Vec<String> = log_lines(&log_of(10), 4, 2).iter().map(line_text).collect();
        assert_eq!(texts, ["line4", "line5", "line6", "line7"]);
    }

    #[test]
    fn rendered_log_pane_puts_the_newest_line_on_the_last_content_row() {
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_log(f, f.area(), &log_of(10), 0, Style::new().fg(Color::Cyan), BorderType::Plain))
            .unwrap();
        let buf = terminal.backend().buffer();
        let row_text = |y: u16| -> String {
            (1..10u16)
                .map(|x| buf.cell((x, y)).unwrap().symbol())
                .collect()
        };
        // 6-row pane: border rows 0 and 5, content rows 1-4.
        assert!(row_text(1).starts_with("line6"), "row1: {}", row_text(1));
        assert!(row_text(4).starts_with("line9"), "row4: {}", row_text(4));
    }

    #[test]
    fn rendered_log_pane_is_bottom_anchored_when_the_log_is_short() {
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_log(f, f.area(), &log_of(1), 0, Style::new().fg(Color::Cyan), BorderType::Plain))
            .unwrap();
        let buf = terminal.backend().buffer();
        let row_text = |y: u16| -> String {
            (1..8u16)
                .map(|x| buf.cell((x, y)).unwrap().symbol())
                .collect()
        };
        assert!(row_text(1).trim().is_empty(), "row1: {}", row_text(1));
        assert!(row_text(4).starts_with("line0"), "row4: {}", row_text(4));
    }

    fn ui_device(key: &str, name: &str, section: crate::ui::types::DeviceSection) -> UiDevice {
        ui_device_shared(key, name, section, true)
    }

    fn ui_device_shared(
        key: &str,
        name: &str,
        section: crate::ui::types::DeviceSection,
        in_shared_config: bool,
    ) -> UiDevice {
        UiDevice {
            key: key.to_string(),
            name: name.to_string(),
            ip: Some("10.0.0.1".to_string()),
            port: 22,
            section,
            in_shared_config,
        }
    }

    /// The Devices pane is one list with a divider row per section, in
    /// the same style: "── localhost ──" + the local device row, then
    /// "── added ──" + the added devices, then "── detected ──" + the
    /// detected devices. The selection index addresses the device rows,
    /// not the dividers.
    #[test]
    fn devices_pane_renders_local_added_detected_with_dividers_in_order() {
        use crate::ui::types::DeviceSection;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let devices = vec![
            ui_device("k-local", "myhost", DeviceSection::Local),
            ui_device("k-a", "alpha", DeviceSection::Added),
            ui_device("k-b", "beta", DeviceSection::Added),
            ui_device("k-d", "gamma", DeviceSection::Detected),
        ];
        terminal
            .draw(|frame| {
                render_all_panes(
                    frame,
                    frame.area(),
                    ActivePane::main_menu(),
                    NavFocus::Center,
                    &devices,
                    Some(2), // "beta"
                    &[],
                    0,
                    &HashMap::new(),
                    &HashMap::new(),
                    &[],
                    None,
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let lines: Vec<String> = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                    .collect()
            })
            .collect();
        let pos = |needle: &str| {
            lines
                .iter()
                .position(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("{needle} not rendered"))
        };
        let p_local_div = pos("── localhost (added) ──");
        let p_local = pos("myhost");
        let p_added_div = pos("── added ──");
        let p_alpha = pos("alpha");
        let p_beta = pos("beta");
        let p_det_div = pos("── detected ──");
        let p_gamma = pos("gamma");
        assert!(
            p_local_div < p_local
                && p_local < p_added_div
                && p_added_div < p_alpha
                && p_alpha < p_beta
                && p_beta < p_det_div
                && p_det_div < p_gamma,
            "rows out of order: localhost_div={p_local_div} local={p_local} added_div={p_added_div} alpha={p_alpha} beta={p_beta} det_div={p_det_div} gamma={p_gamma}"
        );
        assert!(lines.iter().any(|l| l.contains("Devices — 2 added, 1 detected")));
        // The selection index 2 addresses "beta" (dividers are not rows):
        // its row is rendered reversed.
        let beta_row = (0..buf.area.width).any(|x| {
            buf[(x, p_beta as u16)]
                .modifier
                .contains(Modifier::REVERSED)
        });
        assert!(beta_row, "selected device row must be highlighted");
        assert!(!buf[(0, p_local as u16)]
            .modifier
            .contains(Modifier::REVERSED));
    }

    #[test]
    fn localhost_divider_shows_shared_config_membership() {
        use crate::ui::types::DeviceSection;
        for (in_shared, needle) in [
            (true, "── localhost (added) ──"),
            (false, "── localhost (not added) ──"),
        ] {
            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            let devices = vec![ui_device_shared("k-local", "myhost", DeviceSection::Local, in_shared)];
            terminal
                .draw(|frame| {
                    render_all_panes(
                        frame,
                        frame.area(),
                        ActivePane::main_menu(),
                        NavFocus::Center,
                        &devices,
                        Some(0),
                        &[],
                        0,
                        &HashMap::new(),
                        &HashMap::new(),
                        &[],
                        None,
                    );
                })
                .unwrap();
            let buf = terminal.backend().buffer();
            let lines: Vec<String> = (0..buf.area.height)
                .map(|y| {
                    (0..buf.area.width)
                        .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                        .collect()
                })
                .collect();
            assert!(
                lines.iter().any(|l| l.contains(needle)),
                "divider {needle} not rendered for in_shared_config={in_shared}"
            );
        }
    }

    #[test]
    fn users_pane_pins_current_user_then_added_then_not_added() {
        use crate::ui::types::{UiUser, UserStatus};
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let users = vec![
            UiUser { name: "matthias".to_string(), status: UserStatus::KeyInstalled, added: true, pinned: true, is_default: false, incoming: None },
            UiUser { name: "alice".to_string(), status: UserStatus::Confirmed, added: true, pinned: false, is_default: false, incoming: None },
            UiUser { name: "bob".to_string(), status: UserStatus::Temporary, added: false, pinned: false, is_default: false, incoming: None },
        ];
        terminal
            .draw(|frame| {
                render_users_panel(frame, frame.area(), &users, Some(2), true);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let lines: Vec<String> = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                    .collect()
            })
            .collect();
        let pos = |needle: &str| {
            lines
                .iter()
                .position(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("{needle} not rendered"))
        };
        let p_cur_div = pos("── current user (added) ──");
        let p_matthias = pos("matthias");
        let p_added_div = pos("── added ──");
        let p_alice = pos("alice");
        let p_na_div = pos("── not added ──");
        let p_bob = pos("bob");
        assert!(
            p_cur_div < p_matthias
                && p_matthias < p_added_div
                && p_added_div < p_alice
                && p_alice < p_na_div
                && p_na_div < p_bob,
            "users out of order: {p_cur_div} {p_matthias} {p_added_div} {p_alice} {p_na_div} {p_bob}"
        );
        // The selection index addresses user rows, not dividers: index 2
        // highlights "bob".
        let bob_row = (0..buf.area.width)
            .any(|x| buf[(x, p_bob as u16)].modifier.contains(Modifier::REVERSED));
        assert!(bob_row, "selected user row must be highlighted");
        assert!(!buf[(0, p_matthias as u16)]
            .modifier
            .contains(Modifier::REVERSED));
    }

    /// The device's default user is the pinned first row, under its own
    /// `── default ──` divider, ahead of the membership groups; the divider
    /// alone marks the row (the status column keeps its plain label). The
    /// selection index addresses the user rows, not the dividers.
    #[test]
    fn users_pane_pins_default_user_under_its_own_divider() {
        use crate::ui::types::{UiUser, UserStatus};
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let users = vec![
            UiUser { name: "alice".to_string(), status: UserStatus::Confirmed, added: true, pinned: false, is_default: true, incoming: None },
            UiUser { name: "matthias".to_string(), status: UserStatus::Confirmed, added: true, pinned: true, is_default: false, incoming: None },
            UiUser { name: "bob".to_string(), status: UserStatus::Temporary, added: false, pinned: false, is_default: false, incoming: None },
        ];
        terminal
            .draw(|frame| {
                render_users_panel(frame, frame.area(), &users, Some(1), true);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let lines: Vec<String> = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                    .collect()
            })
            .collect();
        let pos = |needle: &str| {
            lines
                .iter()
                .position(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("{needle} not rendered"))
        };
        let p_def_div = pos("── default ──");
        let p_alice = pos("alice");
        let p_cur_div = pos("── current user (added) ──");
        let p_matthias = pos("matthias");
        let p_bob = pos("bob");
        assert!(
            p_def_div < p_alice
                && p_alice < p_cur_div
                && p_cur_div < p_matthias
                && p_matthias < p_bob,
            "users out of order: {p_def_div} {p_alice} {p_cur_div} {p_matthias} {p_bob}"
        );
        // The divider marks the default row; the status column does not repeat it.
        assert!(lines[p_alice].contains("confirmed"), "alice row: {}", lines[p_alice]);
        assert!(!lines[p_alice].contains("default"), "alice row: {}", lines[p_alice]);
        // Selection index 1 addresses "matthias" even with two divider rows above.
        let matthias_row = (0..buf.area.width)
            .any(|x| buf[(x, p_matthias as u16)].modifier.contains(Modifier::REVERSED));
        assert!(matthias_row, "selected user row must be highlighted");
        assert!(!buf[(0, p_alice as u16)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(0, p_def_div as u16)].modifier.contains(Modifier::REVERSED));
    }
}
