# Navigation Model

This document is the reference specification for the full-screen TUI
navigation. It describes the target model; the code is being migrated to it.

## Principles

The user is **virtually positioned at the center of the screen** when no
panel is engaged. Arrows move the user **directionally**: a key points toward
the panel in that direction, and each further press in the same direction
goes one step deeper. The opposite arrow rewinds one step at a time.

- **Arrows** — primary navigation: direction toward a panel, then deeper
  into it (selection, fullscreen, sub-panel). Inside a list, `Up`/`Down`
  move the selection and **wrap around** (loop) at the ends.
- **`Enter`** — last resort only: enter a sub-context when the arrows are
  already consumed by the content (e.g. zooming the fullscreen logs).
- **`Esc`** — last resort only: go back one level when the arrows are
  already consumed by the content.

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
│   └── (device rows; Up/Dn select, wrapping)
├── Added panel (right)
│   └── machine rows (Up/Dn select, wrapping)
│       └── Users sub-panel   ← nested INSIDE the Added panel
│           └── (user rows; Up/Dn select, wrapping)
└── Activity panel (bottom)
    └── fullscreen view (scrolling)
```

The **Users sub-panel is part of the Added panel**, not a sibling panel. It
is a sub-context of the selected machine, reached by continuing in the same
direction (`Right` from the Added panel); `Left` returns to the machine
level. There is no lateral panel switch to reach it.

`Right` always enters the Users selection mode, whatever the machine's user
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
| `Right` | Added panel **fullscreen**, machine list active (first machine selected) |
| `Right` | **Users sub-panel** of the selected machine (first user selected) |
| `Left` | Back to the machine list (Users left) |
| `Left` | Back to Added selected |
| `Left` | Back to the center, overview |

Symmetrically, `Left` from the center selects the Detected panel, and
further presses walk into it; `Right` rewinds.

### Bottom panel (Activity) and fullscreen zoom

| Press | State |
|-------|-------|
| `Down` | Activity panel **selected** |
| `Down` | Activity **fullscreen** |
| `Up` | Back to Activity selected (fullscreen left) |
| `Up` | Back to the center, overview |

### Exception: fullscreen logs

Once the Activity panel is fullscreen, `Up`/`Down` (and `PgUp`/`PgDn`)
scroll the content — the arrow cannot be used to zoom out anymore. This is
where the last-resort keys apply:

- `Enter` — expand the logs fullscreen
- `Esc` — collapse back to the dashboard

### List selection

Inside a fullscreen panel (machines, users, detected devices), `Up`/`Down`
move the selection and **wrap around**: past the last item the selection
loops back to the first, and vice versa. The selection never leaves its
list; changing panels or zooming is done with the directional presses
described above.

## Summary Table

| Key | Meaning |
|-----|---------|
| `Up`/`Down`/`Left`/`Right` | Walk directionally: overview → panel selected → fullscreen → sub-panel; opposite key rewinds |
| `Up`/`Down` (in a list) | Move selection, wrapping around at the ends |
| `Enter` | Last resort: enter a sub-context when arrows are consumed (e.g. logs fullscreen zoom) |
| `Esc` | Last resort: go back one level when arrows are consumed |
| `PgUp`/`PgDn` | Scroll scrollable content (activity pane, help) |

Character shortcuts (`q` quit, `s` sync, `d` delete, `w` wipe, `c` setup-cd,
`k` connections, `v` verbose, `?` help) remain unchanged; they act as
commands, not navigation.
