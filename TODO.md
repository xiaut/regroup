# TODO

## 1. Users pane: localhost and added devices only

The Users sub-panel should be reachable on the **localhost** row and on
**added** device rows — never on a *detected* row (a pending device has
no users yet; that part already holds).

Current state:

- `Navigation::handle_key` (`src/ui/types.rs`) opens the Users sub-panel
  from the Devices pane only when the selected row is
  `Some(DeviceSection::Added)`; `DEVICES_LOCAL_KEYS` has no `Enter`/`Right`
  "users" key, so the localhost row cannot open it.
- `InteractiveState::users_view` / `selected_added_device`
  (`src/interactive/state.rs`) resolve users only for `DeviceRow::Added`.

To do:

- Add `Enter`/`Right` → users on the local row (`DEVICES_LOCAL_KEYS`) and
  let `Navigation::handle_key` accept `DeviceSection::Local`.
- Extend `selected_added_device` / `users_view` to also serve
  `DeviceRow::Local`, loading the local device's own users
  (`devices/<local_name>/users/`, written by `ensure_local_user_file` in
  `src/config/users.rs`; current user from `discovery::local_user()`).
- Keep detected rows without a Users pane.

## 2. Question: is the temporary CLI pass useless on some actions?

`Ui::with_clean_terminal` (`src/ui/interface.rs:152`) leaves the
alternate screen, enables direct output, logs `TUI -> CLI (<action>)` /
`CLI -> TUI (<action>)`, and pauses with "Press Enter to return" on
**every** call. Call sites (`src/interactive/state.rs`):

| action label        | line | runs a subprocess whose output must be seen live? |
|---------------------|------|----------------------------------------------------|
| `git sync`          | 688  | yes (git pull/push/commit)                         |
| `trust new signing key` | 714 | probably only file writes + `ui::log`          |
| `wipe`              | 814  | question — mostly deletions                        |
| `remove device`     | 967, 1435 | question — file/ssh-config edits, maybe git  |
| `add device`        | 1090 | question — file writes, maybe git commit           |
| `install SSH key`   | 1316 | yes (ssh / receive-install-key)                    |
| `remove SSH key`    | 1389 | yes (ssh / receive-remove-key)                     |

Question: for actions that only log through `ui::log` and spawn no
subprocess, the whole pass — leaving the TUI, the Enter pause,
re-entering — is pure friction. Should the pass be reserved for actions
with real direct output (sync, key install/remove), keeping the log
markers but skipping the screen switch and the pause? Also note the
inconsistency: `SetupCdConfirm` is *not* wrapped in a pass at all, even
though it writes to the shell rc file.

## 3. Default user per device

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
