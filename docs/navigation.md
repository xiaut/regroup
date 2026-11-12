# Navigation Model

This document is the reference specification for the full-screen TUI
navigation. It describes the target model; the code is being migrated to it.

## Principles

The interface is a hierarchy of **contexts**. Navigation is uniform:

- **`Esc`** — go back one level in the context hierarchy (leave a
  sub-context, close a mode, deselect).
- **`Enter`** — enter the sub-context of the focused item (machine → its
  users, panel → fullscreen, list → detail).
- **Arrows** — move focus within the current context, and between sibling
  panels when the current context has no further item in that direction.

The same physical action always has the same meaning at the same level; the
user never needs to learn per-panel navigation keys.

## Context Hierarchy

```
Main screen (all panels visible)
├── Detected panel
│   └── (device rows; Up/Dn select)
├── Added panel (known devices)
│   └── machine row
│       └── Users sub-panel   ← nested INSIDE the Added panel
│           └── (user rows; Up/Dn select)
└── Activity panel
    └── fullscreen view (scrolling)
```

The **Users sub-panel is part of the Added panel**, not a sibling panel. It is
a sub-context of the selected machine: `Enter` on a machine enters it, `Esc`
returns to the machine level. There is no lateral panel switch to reach it.

## Panel Focus and Fullscreen via Arrows

`Up`/`Down` move focus between the stacked zones of the main screen
(Detected → Added → Activity). Pushing focus further in the same direction
zooms:

| State | `Down` | `Up` |
|-------|--------|------|
| No panel focused | Focus the first panel (Detected) | — |
| Detected focused | Focus the panel below (Added) | Deselect focus |
| Added focused | Focus the panel below (Activity) | Focus the panel above (Detected) |
| Activity focused | Expand Activity fullscreen | Focus the panel above (Added) |
| Activity fullscreen | — (scroll) | Collapse fullscreen back to focused |

So the fullscreen toggle is just "one more arrow": e.g. `Down`, `Down` on the
bottom panel makes it fullscreen; `Up`, `Up` restores the layout and then
drops the selection. The same applies symmetrically to the other panels when
they support a fullscreen view.

## Exceptions: Esc/Enter Instead of Arrows

When arrows already carry a meaning inside a context, the fullscreen
enter/exit must not steal them; use `Enter`/`Esc` there instead.

The canonical case is the **Activity (logs) panel fullscreen**: once
fullscreen, `Up`/`Down` (and `PgUp`/`PgDn`) scroll the log content, so
`Enter` expands it and `Esc` collapses it back. In contexts where arrows
only move selection (device lists), arrows alone drive zoom as described
above.

Rule of thumb: inside a context, arrows belong to the content; crossing
context boundaries is done by pushing the arrow past the end of the content
— unless the content consumes the arrow in a open-ended way (scrolling), in
which case `Enter`/`Esc` are the boundary keys.

## Entering Sub-Contexts

| Context | `Enter` | `Esc` |
|---------|---------|-------|
| Added panel, machine selected | Enter the machine's Users sub-panel (first user selected) | Leave the panel / cancel current mode |
| Users sub-panel | Confirm the action on the selected user (e.g. install/remove SSH key) | Back to the machine level |
| Detected panel, device selected | Add the device | — |
| Any confirmation prompt | Confirm | Cancel |

## Summary Table

| Key | Meaning |
|-----|---------|
| `Esc` | Back one level in the context hierarchy |
| `Enter` | Enter the focused sub-context / confirm |
| `Up`/`Down` | Move focus within the context, then between panels, then zoom/unzoom fullscreen |
| `PgUp`/`PgDn` | Scroll scrollable content (activity pane, help) |

Character shortcuts (`q` quit, `s` sync, `d` delete, `w` wipe, `c` setup-cd,
`k` connections, `v` verbose, `?` help) remain unchanged; they act as
commands, not navigation.
