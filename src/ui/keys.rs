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
    pane_keys, Action, Effect, Hint, KeyBinding, NavDepth, NavFocus, QuestionDef, HELP_BINDINGS,
    HELP_KEY,
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
        confirmed_user: bool,
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
            confirmed_user,
        } => {
            let mut bindings = pane_keys(focus, depth, confirmed_user).to_vec();
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
        let mut contexts = Vec::new();
        for focus in [NavFocus::Center, NavFocus::Detected, NavFocus::Added, NavFocus::Users, NavFocus::Logs] {
            for depth in [None, Some(NavDepth::Selected), Some(NavDepth::Fullscreen)] {
                for confirmed_user in [false, true] {
                    contexts.push(Context::Pane { focus, depth, confirmed_user });
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
            Question::Idle(_) => Context::Pane { focus, depth, confirmed_user: true },
            q if matches!(q, Question::Rename(..) | Question::AddTempUser(..)) => Context::TextInput(q.def()),
            q => Context::Modal(q.def()),
        }
    }

    #[test]
    fn every_advertised_key_is_handled() {
        for q in all_questions() {
            for focus in [NavFocus::Center, NavFocus::Detected, NavFocus::Added, NavFocus::Users, NavFocus::Logs] {
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
        let overview = Context::Pane { focus: NavFocus::Center, depth: None, confirmed_user: false };
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
    fn help_is_the_only_global_key() {
        // '?' is advertised and handled in every pane state.
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
    fn unadvertised_key_is_not_handled() {
        // 's' syncs in the Added fullscreen pane, but not in Detected fullscreen.
        let detected_fs = Context::Pane { focus: NavFocus::Detected, depth: Some(NavDepth::Fullscreen), confirmed_user: false };
        assert!(!bindings(detected_fs).iter().any(|b| b.key == KeyCode::Char('s')));
        assert_eq!(resolve(detected_fs, KeyCode::Char('s')), None);

        let added_fs = Context::Pane { focus: NavFocus::Added, depth: Some(NavDepth::Fullscreen), confirmed_user: false };
        assert_eq!(
            resolve(added_fs, KeyCode::Char('s')),
            Some(KeyResolution::Action(Action::Sync))
        );
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
            let Context::Pane { focus, depth, .. } = ctx else { continue };
            let nav = Navigation { focus, depth };
            for code in candidates {
                let nav_changed = nav.handle_key(code, true) != nav;
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
    fn help_panel_only_handles_its_own_keys() {
        let ctx = Context::Help;
        // 's' is not advertised while the help panel is open, so it does nothing.
        assert_eq!(resolve(ctx, KeyCode::Char('s')), None);
        assert_eq!(resolve(ctx, KeyCode::Char('q')), Some(KeyResolution::Internal));
        assert_eq!(resolve(ctx, KeyCode::Esc), Some(KeyResolution::Internal));
    }
}
