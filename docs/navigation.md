# Navigation Model

This document is the reference specification for the full-screen TUI
navigation. It describes the target model; the code is being migrated to it.

## Principles

The user is **virtually positioned at the center of the screen** when no
panel is engaged. Arrows move the user **directionally**: a key points toward
the panel in that direction, and each further press in the same direction
goes one step deeper. The opposite arrow rewinds one step at a time.

- **`Enter` / `Esc`** — always offered, in addition to the arrows, and
  listed first wherever keys are shown. `Enter` goes one level deeper in
  the context hierarchy, `Esc` goes one level back. They are the
  guaranteed, always-available keys; the arrows are the ergonomic path.
- **Arrows** — primary navigation: direction toward a panel, then deeper
  into it (selection, fullscreen, sub-panel). Inside a list, `Up`/`Down`
  move the selection and **wrap around** (loop) at the ends.

Navigation is the deliberate exception to the project rule "one action,
one key": every navigation step is reachable both via the arrows and via
`Enter`/`Esc`, except where an arrow is consumed by the content (e.g.
scrolling the fullscreen Logs, where `Esc` is the only way out).

Navigation is deliberately decomposed into small, explicit steps: selecting
a panel does nothing by itself beyond highlighting it; each press has one
visible effect.

## Panel States

A panel has exactly **three states**:

1. **Overview** — the dashboard is displayed, no panel engaged (user at the
   virtual center).
2. **Selected** — the panel is highlighted (focus), nothing more.
3. **Fullscreen** — the panel fills the main area and its content becomes
   navigable (list selection, scrolling). The Logs pane stays visible as a
   strip above the action block, except for the Logs pane itself, which
   fills the main area, and while the help panel is open.

The same arrow key walks through these states in order; the opposite arrow
walks back.

## Context Hierarchy

```
Main screen (dashboard overview)
├── Devices panel (top, full width)
│   └── one list, each section under its divider:
│       ── localhost ── → local device row → ── added ── → added devices
│       → ── detected ── → detected devices (Up/Down select, wrapping)
│       └── Users sub-panel   ← nested INSIDE the Devices panel
│           └── (user rows; Up/Down select, wrapping)
└── Logs panel (bottom)
    └── fullscreen view (scrolling)
```

The **Devices panel is a single list**, each section under its divider
row, in the same style: `── localhost ──` with the local device row (the
one regroup runs on), then `── added ──` with the added (known) devices,
then `── detected ──` with the detected (pending) ones. The panel sits
**above** the virtual center, so `Up` opens it from the overview.

The **Users sub-panel is part of the Devices panel**, not a sibling panel.
It is a sub-context of the selected *added* device, reached with `Right`
or `Enter` from the Devices panel fullscreen; `Left` or `Esc` returns to
the device level. There is no lateral panel switch to reach it.

`Right`/`Enter` enter the Users selection mode only when the selected row
is an added device (a detected row has no users yet). Whatever the device's user
list looks like — even a single user, or none — entering the mode from an
added device always works. Entering the mode is what unlocks the associated
user actions; it is never skipped because the list is short or empty.

## Scenarios

### Devices panel (top)

The Devices panel is above the center, so the vertical axis walks into
it, one step per press:

| Press | State |
|-------|-------|
| `Up` | Devices panel **selected** (highlighted only) |
| `Up`, `Right` or `Enter` | Devices panel **fullscreen**, the row list active (first row = the local device selected) |
| `Right` or `Enter` | **Users sub-panel** of the selected added device (first user selected) — only when an added device row is selected |
| `Left` (or `Esc`) | Back to Devices selected (from fullscreen, "unbig") |
| `Down` (or `Esc`) | Back to the center, overview (from selected) |
| `Left`/`Esc` | From the Users sub-panel: back to the Devices panel fullscreen |

### Bottom panel (Logs) and fullscreen zoom

| Press | State |
|-------|-------|
| `Down` | Logs panel **selected** |
| `Down` | Logs **fullscreen** |
| `Up` | Back to Logs selected (fullscreen left) |
| `Up` | Back to the center, overview |

### Exception: fullscreen logs

Once the Logs panel is fullscreen, `Up`/`Down` (and `PgUp`/`PgDn`)
scroll the content — the arrow cannot be used to zoom out anymore. This is
where the fallback keys apply: **`Esc` is the only way to leave the
fullscreen Logs panel** (collapse back to the dashboard).

Every view of the Logs pane (overview, strip under a fullscreen device
pane, fullscreen) shows the **tail** of the log — the newest lines on the
pane's last row. Scrolling up exists only in the fullscreen Logs panel;
leaving it does not carry the scroll over, so the other views always
follow the tail.

### List selection

Inside a fullscreen panel (devices, users, detected devices), `Up`/`Down`
move the selection and **wrap around**: past the last item the selection
loops back to the first, and vice versa. The selection never leaves its
list; changing panels or zooming is done with the directional presses
described above.

## Summary Table

| Key | Meaning |
|-----|---------|
| `Enter` | Go one level deeper in the context hierarchy (always available; listed first) |
| `Esc` | Go one level back (always available; listed first; only exit from the fullscreen Logs panel) |
| `Up`/`Down`/`Left`/`Right` | Walk directionally: overview → panel selected → fullscreen → sub-panel; opposite key rewinds |
| `Up`/`Down` (in a list) | Move selection, wrapping around at the ends |
| `PgUp`/`PgDn` | Scroll scrollable content (Logs pane, help) |

Character shortcuts act as commands, not navigation, and are listed per
pane state (see `pane_keys` in `src/ui/types.rs`): `s` sync, `d` delete,
`w` wipe, `c` setup-cd and `Enter`/`Right` into Users when an **added** row
is selected in the Devices panel, `a`/`n` when a **detected** row is
selected, and so on. `q` quit is only offered on the main menu (overview);
`?` help is the single key that works in every pane state.
