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
3. **Fullscreen** — the panel fills the screen and its content becomes
   navigable (list selection, scrolling).

The same arrow key walks through these states in order; the opposite arrow
walks back.

## Context Hierarchy

```
Main screen (dashboard overview)
├── Detected panel (left)
│   └── (device rows; Up/Down select, wrapping)
├── Added panel (right)
│   └── device rows (Up/Down select, wrapping)
│       └── Users sub-panel   ← nested INSIDE the Added panel
│           └── (user rows; Up/Down select, wrapping)
└── Logs panel (bottom)
    └── fullscreen view (scrolling)
```

The **Users sub-panel is part of the Added panel**, not a sibling panel. It
is a sub-context of the selected device, reached by continuing in the same
direction (`Right` from the Added panel); `Left` returns to the device
level. There is no lateral panel switch to reach it.

`Right` always enters the Users selection mode, whatever the device's user
list looks like — even a single user, or none. Entering the mode is what
unlocks the associated user actions; it is never skipped because the list
is short or empty.

## Scenarios

### Side panels (Detected left, Added right)

Starting from the center (overview), pressing `Right` walks toward the
right panel, one step per press:

| Press | State |
|-------|-------|
| `Right` | Added panel **selected** (highlighted only) |
| `Right` | Added panel **fullscreen**, device list active (first device selected) |
| `Right` | **Users sub-panel** of the selected device (first user selected) |
| `Left` | Back to the device list (Users left) |
| `Left` | Back to Added selected |
| `Left` | Back to the center, overview |

Symmetrically, `Left` from the center selects the Detected panel, and
further presses walk into it; `Right` rewinds.

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
`w` wipe, `c` setup-cd in the Added panel, `a`/`n` in Detected, `v` in
Logs, and so on. `q` quit is only offered on the main menu (overview);
`?` help is the single key that works in every pane state.
