# TODO

## 1. Default user per device

New concept: each device has **one default user**.

- Auto-set to the only user when the device has exactly one user
  (computed in `users_view` / state init, `src/interactive/state.rs`).
- Displayed **in the first line, like the localhost device, same
  style**: in the Devices pane the local device is the pinned first row
  under the `── localhost ──` divider (`device_rows` / `divider_row`,
  `src/ui/render.rs`); analogously the default user should be the pinned
  first line of the Users pane (or of the device row), same visual style.
- Actions: in the Users pane, a key to **set the selected user as the
  default among the others — replacing the current one** (radio
  semantics: exactly one default per device; setting another swaps it).
  Add the `Action` variant(s) in `src/ui/types.rs` (`Action`,
  `USERS_CONFIRMED_KEYS` / `USERS_TEMP_KEYS`) and handle them in
  `handle_action` (`src/interactive/state.rs`).
- Useful later: default `user@host` for ssh / install-key actions that
  currently require picking a user in the Users pane.
- Open question — persistence: where does the default user live?
  `device.json` is written only by the device itself (storage ownership
  rule, `src/config/device.rs`), and `settings.json` is local-only
  (`src/config/settings.rs`). A default user for an *added* device is a
  local preference; for the localhost device it could go in its own
  `device.json`. Decide the location before implementing
  (`Device` struct in `src/config/mod.rs:21`, `DeviceUser` /
  `load_device_users_merged` in `src/config/users.rs`, `UiUser` /
  `UserStatus` in `src/ui/types.rs`).
