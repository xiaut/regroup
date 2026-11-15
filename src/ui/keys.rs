//! Single-source key dispatch.
//!
//! One function, [`bindings`], maps the ui context to the complete list
//! of keys that work in it. The hint lines and the key dispatch are
//! both built from that list: [`resolve`] looks a key press up in it.
//! A key that is not in the list is not handled at all, so a hint can
//! never advertise a key that does nothing, and a key can never work
//! without being advertised — the two cannot drift apart.

use crossterm::event::KeyCode;

use super::types::{
    pane_keys, Action, Effect, Hint, KeyBinding, NavDepth, NavFocus, PaneKeyCtx, QuestionDef,
    HELP_BINDINGS, HELP_KEY,
};

/// The ui context a key press happens in — the whole key-relevant state.
#[derive(Clone, Copy, Debug)]
pub enum Context<'a> {
    /// The help panel is open: it has its own keys.
    Help,
    /// A modal question (wipe, delete, trust key, …): only the
    /// question's own keys work.
    Modal(&'a QuestionDef),
    /// A text-input question (rename, add temp user): characters are
    /// captured as text; the question's bindings add the control keys
    /// (Enter submits, Esc cancels).
    TextInput(&'a QuestionDef),
    /// A pane state of the dashboard (Idle question). The one global
    /// key, help ('?'), is appended on top of the pane's own keys.
    Pane {
        focus: NavFocus,
        depth: Option<NavDepth>,
        keys_ctx: PaneKeyCtx,
    },
}

/// The complete key list of a context: what the hint lines are built
/// from, and what key presses are looked up in.
pub fn bindings(ctx: Context) -> Vec<KeyBinding> {
    match ctx {
        Context::Help => HELP_BINDINGS.to_vec(),
        Context::Modal(def) | Context::TextInput(def) => def.bindings.to_vec(),
        Context::Pane {
            focus,
            depth,
            keys_ctx,
        } => {
            let mut bindings = pane_keys(focus, depth, keys_ctx).to_vec();
            bindings.push(HELP_KEY);
            bindings
        }
    }
}

/// The action-bar hints of a context, projected from [`bindings`].
/// Entries that share the same description are merged, e.g.
/// `[Enter/Right] fullscreen`.
pub fn hints(ctx: Context) -> Vec<Hint> {
    let pairs: Vec<(&'static str, &'static str)> = bindings(ctx)
        .iter()
        .map(|b| (b.label, b.description))
        .collect();
    merge_hints(&pairs)
}

/// Merge (key, description) pairs into Hints, joining keys that share
/// the same description: ("Left", "Detected") + ("d", "Detected")
/// becomes Hint { keys: "Left/d", description: "Detected" }.
fn merge_hints(actions: &[(&'static str, &'static str)]) -> Vec<Hint> {
    let mut order: Vec<&'static str> = Vec::new();
    let mut map: Vec<(&'static str, Vec<&'static str>)> = Vec::new();
    for &(key, desc) in actions {
        if let Some(entry) = map.iter_mut().find(|(d, _)| *d == desc) {
            if !entry.1.contains(&key) {
                entry.1.push(key);
            }
        } else {
            map.push((desc, vec![key]));
            order.push(desc);
        }
    }
    order
        .iter()
        .map(|desc| {
            let keys = map.iter().find(|(d, _)| d == desc).unwrap().1.join("/");
            Hint {
                keys,
                description: desc,
            }
        })
        .collect()
}

/// How a key press is resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyResolution {
    /// The key produces this action for the interactive loop.
    Action(Action),
    /// The key submits the text being typed (rename, temporary user).
    SubmitInput,
    /// The key is handled internally by the ui (navigation, scrolling,
    /// help panel, text capture): it produces no action.
    Internal,
}

/// Resolve one key press against the context's key list. Returns `None`
/// when the key is not in the list — such a key is not handled at all.
pub fn resolve(ctx: Context, code: KeyCode) -> Option<KeyResolution> {
    if matches!(ctx, Context::TextInput(_)) {
        return match code {
            KeyCode::Char(_) | KeyCode::Backspace => Some(KeyResolution::Internal),
            _ => lookup(ctx, code),
        };
    }
    lookup(ctx, code)
}

fn lookup(ctx: Context, code: KeyCode) -> Option<KeyResolution> {
    bindings(ctx)
        .iter()
        .find(|b| b.key == code)
        .map(|b| match &b.effect {
            Effect::Action(a) => KeyResolution::Action(a.clone()),
            Effect::SubmitInput => KeyResolution::SubmitInput,
            Effect::Internal => KeyResolution::Internal,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::types::{Navigation, Question};

    fn all_pane_contexts() -> Vec<Context<'static>> {
        use crate::ui::types::DeviceSection;
        let mut contexts = Vec::new();
        for focus in [NavFocus::Center, NavFocus::Devices, NavFocus::Users, NavFocus::Logs] {
            for depth in [None, Some(NavDepth::Selected), Some(NavDepth::Fullscreen)] {
                for selected_section in [None, Some(DeviceSection::Local), Some(DeviceSection::Added), Some(DeviceSection::Detected)] {
                    for local_in_config in [false, true] {
                        for user_external in [false, true] {
                            for has_users in [false, true] {
                                contexts.push(Context::Pane {
                                    focus,
                                    depth,
                                    keys_ctx: PaneKeyCtx { selected_section, local_in_config, user_external, has_users },
                                });
                            }
                        }
                    }
                }
            }
        }
        contexts
    }

    fn all_questions() -> Vec<Question> {
        vec![
            Question::idle(),
            Question::add_device("dev".to_string(), "1.2.3.4".to_string()),
            Question::trust_key("dev".to_string()),
            Question::rename("dev".to_string(), "dev2".to_string()),
            Question::delete("dev".to_string()),
            Question::wipe(2),
            Question::setup_cd("~/.zshrc".to_string()),
            Question::add_temp_user("dev".to_string()),
        ]
    }

    fn context_of(question: &Question, focus: NavFocus, depth: Option<NavDepth>) -> Context<'_> {
        match question {
            Question::Idle(_) => Context::Pane {
                focus,
                depth,
                keys_ctx: PaneKeyCtx { selected_section: None, local_in_config: false, user_external: false, has_users: false },
            },
            q if matches!(q, Question::Rename(..) | Question::AddTempUser(..)) => Context::TextInput(q.def()),
            q => Context::Modal(q.def()),
        }
    }

    #[test]
    fn every_advertised_key_is_handled() {
        for q in all_questions() {
            for focus in [NavFocus::Center, NavFocus::Devices, NavFocus::Users, NavFocus::Logs] {
                for depth in [None, Some(NavDepth::Selected), Some(NavDepth::Fullscreen)] {
                    let ctx = context_of(&q, focus, depth);
                    for b in bindings(ctx) {
                        assert!(
                            resolve(ctx, b.key).is_some(),
                            "advertised key {:?} of {:?} in {:?} is not handled",
                            b.key,
                            q.def().title,
                            ctx
                        );
                    }
                }
            }
        }
        let ctx = Context::Help;
        for b in bindings(ctx) {
            assert!(resolve(ctx, b.key).is_some(), "advertised {:?} not handled", b.key);
        }
    }

    #[test]
    fn q_quits_only_from_the_main_menu() {
        // Overview: 'q' is advertised and quits.
        let overview = Context::Pane {
            focus: NavFocus::Center,
            depth: None,
            keys_ctx: PaneKeyCtx { selected_section: None, local_in_config: false, user_external: false, has_users: false },
        };
        assert!(
            bindings(overview)
                .iter()
                .any(|b| b.key == KeyCode::Char('q') && b.effect == Effect::Action(Action::Quit))
        );
        assert_eq!(
            resolve(overview, KeyCode::Char('q')),
            Some(KeyResolution::Action(Action::Quit))
        );
        // Every other pane state: 'q' is neither advertised nor handled.
        for ctx in all_pane_contexts() {
            if matches!(ctx, Context::Pane { focus: NavFocus::Center, depth: None, .. }) {
                continue;
            }
            assert!(
                !bindings(ctx).iter().any(|b| b.key == KeyCode::Char('q')),
                "q advertised in {:?}",
                ctx
            );
            assert_eq!(resolve(ctx, KeyCode::Char('q')), None, "q handled in {:?}", ctx);
        }
        // Modal questions: 'q' is not handled (except text input, where it
        // is typed into the name).
        for q in all_questions() {
            if matches!(q, Question::Idle(_)) {
                continue;
            }
            let ctx = context_of(&q, NavFocus::Center, None);
            let resolution = resolve(ctx, KeyCode::Char('q'));
            if matches!(q, Question::Rename(..) | Question::AddTempUser(..)) {
                assert_eq!(resolution, Some(KeyResolution::Internal), "q in '{}'", q.def().title);
            } else {
                assert_eq!(resolution, None, "q in '{}'", q.def().title);
            }
        }
    }

    #[test]
    fn help_is_advertised_in_every_pane_state() {
        // '?' is advertised and handled in every pane state — the single
        // key that works everywhere.
        for ctx in all_pane_contexts() {
            assert!(bindings(ctx).iter().any(|b| b.key == KeyCode::Char('?')), "? missing in {:?}", ctx);
            assert_eq!(resolve(ctx, KeyCode::Char('?')), Some(KeyResolution::Internal));
        }
        // Modal questions show only their own keys: no '?'. (In the
        // text-input questions '?' is just a typed character.)
        for q in all_questions() {
            if matches!(q, Question::Idle(_) | Question::AddDevice(..)) {
                continue; // AddDevice is plain-mode only and lists its own keys
            }
            let ctx = context_of(&q, NavFocus::Center, None);
            if matches!(ctx, Context::TextInput(_)) {
                assert_eq!(resolve(ctx, KeyCode::Char('?')), Some(KeyResolution::Internal));
            } else {
                assert!(!bindings(ctx).iter().any(|b| b.key == KeyCode::Char('?')), "? in '{}'", q.def().title);
                assert_eq!(resolve(ctx, KeyCode::Char('?')), None);
            }
        }
    }

    #[test]
    fn global_actions_live_on_the_main_menu_only() {
        use crate::ui::types::DeviceSection;
        let main_menu = Context::Pane {
            focus: NavFocus::Center,
            depth: None,
            keys_ctx: PaneKeyCtx { selected_section: None, local_in_config: false, user_external: false, has_users: false },
        };
        // The global actions (not specific to a device or a user) are
        // advertised and handled on the main menu (overview).
        let globals = [
            (KeyCode::Char('s'), Action::Sync),
            (KeyCode::Char('w'), Action::WipeMode),
            (KeyCode::Char('c'), Action::SetupCdMode),
            (KeyCode::Char('q'), Action::Quit),
        ];
        for (key, action) in &globals {
            assert!(
                bindings(main_menu).iter().any(|b| b.key == *key),
                "{key:?} not advertised on the main menu"
            );
            assert_eq!(
                resolve(main_menu, *key),
                Some(KeyResolution::Action(action.clone())),
                "{key:?} does not resolve on the main menu"
            );
        }
        // ... and nowhere else: with a device row or a user selected, only
        // the device-/user-specific actions (plus navigation and help) are
        // advertised — never the global ones.
        for ctx in all_pane_contexts() {
            let Context::Pane { focus, depth, .. } = ctx else { continue };
            if matches!(focus, NavFocus::Center) && depth.is_none() {
                continue; // the main menu itself
            }
            for (key, _) in &globals {
                assert!(
                    !bindings(ctx).iter().any(|b| b.key == *key),
                    "{key:?} advertised in {ctx:?}"
                );
                assert_eq!(resolve(ctx, *key), None, "{key:?} handled in {ctx:?}");
            }
        }
        // A detected row has no users: no add-user key there either.
        let detected = Context::Pane {
            focus: NavFocus::Devices,
            depth: Some(NavDepth::Selected),
            keys_ctx: PaneKeyCtx {
                selected_section: Some(DeviceSection::Detected),
                local_in_config: false,
                user_external: false,
                has_users: false,
            },
        };
        assert_eq!(resolve(detected, KeyCode::Char('a')), Some(KeyResolution::Action(Action::Add)));
        assert_eq!(resolve(detected, KeyCode::Char('u')), None);
    }

    #[test]
    fn unadvertised_key_is_not_handled() {
        use crate::ui::types::DeviceSection;
        let keys_ctx = |selected_section| PaneKeyCtx { selected_section, local_in_config: false, user_external: false, has_users: false };
        // 's' syncs from the main menu, not from any Devices pane row:
        // a selected device offers only device-specific actions.
        for section in [Some(DeviceSection::Added), Some(DeviceSection::Detected), Some(DeviceSection::Local), None] {
            let ctx = Context::Pane { focus: NavFocus::Devices, depth: Some(NavDepth::Selected), keys_ctx: keys_ctx(section) };
            assert!(!bindings(ctx).iter().any(|b| b.key == KeyCode::Char('s')), "s advertised for section {section:?}");
            assert_eq!(resolve(ctx, KeyCode::Char('s')), None);
        }
    }

    #[test]
    fn local_row_offers_add_or_remove_depending_on_membership() {
        use crate::ui::types::DeviceSection;
        let ctx = |local_in_config| Context::Pane {
            focus: NavFocus::Devices,
            depth: Some(NavDepth::Selected),
            keys_ctx: PaneKeyCtx {
                selected_section: Some(DeviceSection::Local),
                local_in_config,
                user_external: false,
                has_users: true,
            },
        };
        // Not in the shared configuration: 'a' adds it, 'r' does nothing.
        let not_added = ctx(false);
        assert_eq!(
            resolve(not_added, KeyCode::Char('a')),
            Some(KeyResolution::Action(Action::AddLocalToConfig))
        );
        assert_eq!(resolve(not_added, KeyCode::Char('r')), None);
        assert!(bindings(not_added).iter().any(|b| b.key == KeyCode::Char('a')));
        // In the shared configuration: 'r' removes it, 'a'/'d' do nothing
        // ('d' delete stays reserved for added devices).
        let added = ctx(true);
        assert_eq!(
            resolve(added, KeyCode::Char('r')),
            Some(KeyResolution::Action(Action::RemoveLocalFromConfig))
        );
        assert_eq!(resolve(added, KeyCode::Char('a')), None);
        assert_eq!(resolve(added, KeyCode::Char('d')), None);
        // Both variants keep the users entry keys; the global actions
        // (sync, wipe, setup-cd), the added-row delete and any add-user
        // key stay off the local row — a temporary user configures an
        // outgoing connection, and connecting to the current device is
        // nonsense, so the localhost device gets no temp user.
        for c in [ctx(false), ctx(true)] {
            assert!(
                bindings(c)
                    .iter()
                    .any(|b| b.key == KeyCode::Enter && b.effect == Effect::Internal)
            );
            assert_eq!(resolve(c, KeyCode::Char('u')), None);
            assert_eq!(resolve(c, KeyCode::Char('s')), None);
            assert_eq!(resolve(c, KeyCode::Char('w')), None);
            assert_eq!(resolve(c, KeyCode::Char('c')), None);
            assert_eq!(resolve(c, KeyCode::Char('d')), None);
        }
        // The added row offers delete and add-user ('a' is free there: it
        // adds the device on a detected row and the local device on the
        // localhost row), not the globals.
        let added_row = Context::Pane {
            focus: NavFocus::Devices,
            depth: Some(NavDepth::Selected),
            keys_ctx: PaneKeyCtx {
                selected_section: Some(DeviceSection::Added),
                local_in_config: true,
                user_external: false,
                has_users: true,
            },
        };
        assert_eq!(
            resolve(added_row, KeyCode::Char('d')),
            Some(KeyResolution::Action(Action::DeleteSelected))
        );
        assert_eq!(
            resolve(added_row, KeyCode::Char('a')),
            Some(KeyResolution::Action(Action::AddTempUser))
        );
        assert_eq!(resolve(added_row, KeyCode::Char('u')), None);
        assert_eq!(resolve(added_row, KeyCode::Char('s')), None);
        assert_eq!(resolve(added_row, KeyCode::Char('w')), None);
        assert_eq!(resolve(added_row, KeyCode::Char('c')), None);
    }

    #[test]
    fn users_pane_on_localhost_offers_only_set_default() {
        use crate::ui::types::DeviceSection;
        let ctx = Context::Pane {
            focus: NavFocus::Users,
            depth: Some(NavDepth::Selected),
            keys_ctx: PaneKeyCtx {
                selected_section: Some(DeviceSection::Local),
                local_in_config: true,
                user_external: false,
                has_users: true,
            },
        };
        // No install/remove key (connecting to the current device is
        // nonsense) and no remove-user (on localhost you only edit your
        // own data) — only set default, the exception living in the
        // device-owned device.json.
        for key in ['i', 'r', 'x', 'a', 'u', 's', 'w', 'c'] {
            assert_eq!(resolve(ctx, KeyCode::Char(key)), None, "{key} handled on a localhost user row");
            assert!(!bindings(ctx).iter().any(|b| b.key == KeyCode::Char(key)), "{key} advertised on a localhost user row");
        }
        assert_eq!(
            resolve(ctx, KeyCode::Char('d')),
            Some(KeyResolution::Action(Action::SetDefaultUser))
        );
    }

    #[test]
    fn users_pane_on_added_device_offers_the_same_keys_on_every_user_row() {
        use crate::ui::types::DeviceSection;
        // "Temporary" is not a special kind of user: a user added with
        // 'a' is temporary until a sync or a successful key install
        // confirms it, so the Users pane offers the same actions on every
        // user row — the user layer (d set default, x remove user) and
        // the key layer on top (i/r install/remove key).
        let ctx = Context::Pane {
            focus: NavFocus::Users,
            depth: Some(NavDepth::Selected),
            keys_ctx: PaneKeyCtx {
                selected_section: Some(DeviceSection::Added),
                local_in_config: true,
                user_external: false,
                has_users: true,
            },
        };
        let expected = [
            (KeyCode::Char('d'), Action::SetDefaultUser),
            (KeyCode::Char('i'), Action::InstallKeyToSelected),
            (KeyCode::Char('r'), Action::RemoveKeyFromSelected),
            (KeyCode::Char('x'), Action::RemoveUser),
        ];
        for (key, action) in &expected {
            assert_eq!(resolve(ctx, *key), Some(KeyResolution::Action(action.clone())));
            assert!(bindings(ctx).iter().any(|b| b.key == *key));
        }
        // No globals, no device-level add-user in the Users pane.
        for key in ['s', 'w', 'c', 'a', 'u'] {
            assert_eq!(resolve(ctx, KeyCode::Char(key)), None, "{key} handled in Users");
            assert!(!bindings(ctx).iter().any(|b| b.key == KeyCode::Char(key)), "{key} advertised in Users");
        }
    }

    #[test]
    fn external_row_in_localhost_users_pane_offers_only_revoke() {
        use crate::ui::types::DeviceSection;
        let ctx = Context::Pane {
            focus: NavFocus::Users,
            depth: Some(NavDepth::Selected),
            keys_ctx: PaneKeyCtx {
                selected_section: Some(DeviceSection::Local),
                local_in_config: true,
                user_external: true,
                has_users: true,
            },
        };
        // 'r' revokes the incoming connection (a local operation on this
        // device); the other user actions (set default, install/remove
        // key, remove temp user) and the globals are not offered.
        assert_eq!(
            resolve(ctx, KeyCode::Char('r')),
            Some(KeyResolution::Action(Action::RevokeIncomingKey))
        );
        assert!(bindings(ctx).iter().any(|b| b.key == KeyCode::Char('r')));
        for key in ['d', 'i', 'x', 's', 'w', 'c', 'a', 'u'] {
            assert_eq!(resolve(ctx, KeyCode::Char(key)), None, "{key} handled on external row");
            assert!(!bindings(ctx).iter().any(|b| b.key == KeyCode::Char(key)), "{key} advertised on external row");
        }
        // Navigation still works (Up/Down are internal in the table; the
        // list-selection action is derived from them by the ui).
        assert_eq!(resolve(ctx, KeyCode::Esc), Some(KeyResolution::Internal));
        assert_eq!(resolve(ctx, KeyCode::Up), Some(KeyResolution::Internal));
    }

    #[test]
    fn navigation_keys_are_in_the_table() {
        // Every key the navigation state machine reacts to must be
        // advertised (as internal) in the pane's key list.
        let candidates = [
            KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right,
            KeyCode::Enter, KeyCode::Esc, KeyCode::PageUp, KeyCode::PageDown,
        ];
        for ctx in all_pane_contexts() {
            let Context::Pane { focus, depth, keys_ctx } = ctx else { continue };
            let nav = Navigation { focus, depth };
            for code in candidates {
                let nav_changed = nav.handle_key(code, keys_ctx.selected_section, keys_ctx.has_users) != nav;
                let list_nav = nav.list_nav_action(code).is_some();
                if nav_changed || list_nav {
                    assert!(
                        bindings(ctx).iter().any(|b| b.key == code && b.effect == Effect::Internal),
                        "nav key {:?} of {:?} not advertised",
                        code,
                        ctx
                    );
                }
            }
        }
    }

    #[test]
    fn users_entry_keys_withheld_when_the_device_has_no_users() {
        use crate::ui::types::DeviceSection;
        // With no users on the selected device the Users pane is not
        // reachable: Right/Enter are neither advertised nor handled on
        // the local or an added row, and the navigation does not move.
        for section in [Some(DeviceSection::Local), Some(DeviceSection::Added)] {
            let no_users = Context::Pane {
                focus: NavFocus::Devices,
                depth: Some(NavDepth::Selected),
                keys_ctx: PaneKeyCtx {
                    selected_section: section,
                    local_in_config: true,
                    user_external: false,
                    has_users: false,
                },
            };
            let nav = Navigation { focus: NavFocus::Devices, depth: Some(NavDepth::Selected) };
            for key in [KeyCode::Enter, KeyCode::Right] {
                assert!(
                    !bindings(no_users).iter().any(|b| b.key == key),
                    "{key:?} advertised without users (section {section:?})"
                );
                assert_eq!(resolve(no_users, key), None, "{key:?} handled without users (section {section:?})");
                assert_eq!(nav.handle_key(key, section, false), nav, "{key:?} enters Users without users");
            }
            // With users, the entry keys come back: advertised, handled,
            // and moving the navigation into the Users pane.
            let with_users = Context::Pane {
                focus: NavFocus::Devices,
                depth: Some(NavDepth::Selected),
                keys_ctx: PaneKeyCtx {
                    selected_section: section,
                    local_in_config: true,
                    user_external: false,
                    has_users: true,
                },
            };
            for key in [KeyCode::Enter, KeyCode::Right] {
                assert!(bindings(with_users).iter().any(|b| b.key == key && b.effect == Effect::Internal));
                assert_eq!(resolve(with_users, key), Some(KeyResolution::Internal));
                assert_eq!(
                    nav.handle_key(key, section, true),
                    Navigation { focus: NavFocus::Users, depth: Some(NavDepth::Selected) }
                );
            }
        }
        // A detected row has no users entry either way.
        let detected = Context::Pane {
            focus: NavFocus::Devices,
            depth: Some(NavDepth::Selected),
            keys_ctx: PaneKeyCtx {
                selected_section: Some(DeviceSection::Detected),
                local_in_config: false,
                user_external: false,
                has_users: false,
            },
        };
        assert_eq!(resolve(detected, KeyCode::Enter), None);
        assert_eq!(resolve(detected, KeyCode::Right), None);
    }

    #[test]
    fn help_panel_only_handles_its_own_keys() {
        let ctx = Context::Help;
        // 's' is not advertised while the help panel is open, so it does nothing.
        assert_eq!(resolve(ctx, KeyCode::Char('s')), None);
        // Esc is the only key that closes the help panel: '?' and 'q'
        // are not in HELP_BINDINGS, so they are not handled here ('?'
        // only opens the panel from a pane, via HELP_KEY).
        assert_eq!(resolve(ctx, KeyCode::Char('?')), None);
        assert_eq!(resolve(ctx, KeyCode::Char('q')), None);
        assert_eq!(resolve(ctx, KeyCode::Esc), Some(KeyResolution::Internal));
    }
}
