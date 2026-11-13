//! Single-source key dispatch.
//!
//! The hint lines and the key handling are built from the same table:
//! [`bindings_for`] returns the keys of the current ui state, and
//! [`resolve`] looks a key press up in exactly that table. A key that is
//! not in the table is not handled at all, so a hint can never advertise
//! a key that does nothing, and a key can never work without being
//! advertised — the two cannot drift apart.

use crossterm::event::KeyCode;

use super::types::{
    Action, Effect, KeyBinding, Navigation, Question, GLOBAL_BINDINGS, HELP_BINDINGS,
};

/// The ui state a key press is resolved against.
pub struct KeyContext<'a> {
    pub navigation: Navigation,
    pub question: &'a Question,
    /// Whether the help panel is currently shown (it has its own keys).
    pub show_help: bool,
    pub has_users: bool,
    pub confirmed_user: bool,
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

/// The key table of the current state: what the hint lines are built
/// from, and what key presses are looked up in.
pub fn bindings_for(ctx: &KeyContext) -> Vec<KeyBinding> {
    if ctx.show_help {
        return HELP_BINDINGS.to_vec();
    }
    match ctx.question {
        Question::Idle(_) => ctx.navigation.bindings(ctx.has_users, ctx.confirmed_user),
        // While typing a name, character keys are captured as text, so
        // the global 'q' (quit) does not apply: only the question's own
        // bindings (Enter, Esc) are offered.
        Question::Rename(..) | Question::AddTempUser(..) => ctx.question.def().bindings.to_vec(),
        q => {
            let mut bindings = q.def().bindings.to_vec();
            bindings.extend(GLOBAL_BINDINGS.iter().cloned());
            bindings
        }
    }
}

/// Resolve one key press against the state's table. Returns `None` when
/// the key is not in the table — such a key is not handled at all.
pub fn resolve(ctx: &KeyContext, code: KeyCode) -> Option<KeyResolution> {
    if !ctx.show_help
        && matches!(ctx.question, Question::Rename(..) | Question::AddTempUser(..))
    {
        return match code {
            KeyCode::Char(_) | KeyCode::Backspace => Some(KeyResolution::Internal),
            _ => lookup(ctx, code),
        };
    }
    lookup(ctx, code)
}

fn lookup(ctx: &KeyContext, code: KeyCode) -> Option<KeyResolution> {
    bindings_for(ctx)
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
    use crate::ui::types::{NavDepth, NavFocus};

    fn all_nav_states() -> Vec<Navigation> {
        let mut states = Vec::new();
        for focus in [NavFocus::Center, NavFocus::Detected, NavFocus::Added, NavFocus::Users, NavFocus::Logs] {
            for depth in [None, Some(NavDepth::Selected), Some(NavDepth::Fullscreen)] {
                states.push(Navigation { focus, depth });
            }
        }
        states
    }

    fn make_ctx<'a>(navigation: Navigation, question: &'a Question) -> KeyContext<'a> {
        KeyContext {
            navigation,
            question,
            show_help: false,
            has_users: true,
            confirmed_user: true,
        }
    }

    #[test]
    fn every_advertised_idle_key_is_handled() {
        let idle = Question::idle();
        for nav in all_nav_states() {
            for confirmed_user in [false, true] {
                let ctx = KeyContext {
                    navigation: nav,
                    question: &idle,
                    show_help: false,
                    has_users: true,
                    confirmed_user,
                };
                for b in bindings_for(&ctx) {
                    assert!(
                        resolve(&ctx, b.key).is_some(),
                        "advertised key {:?} is not handled in state {:?}",
                        b.key,
                        nav
                    );
                }
            }
        }
    }

    #[test]
    fn q_is_advertised_and_quits_in_every_idle_state() {
        let idle = Question::idle();
        for nav in all_nav_states() {
            let ctx = make_ctx(nav, &idle);
            assert!(
                bindings_for(&ctx).iter().any(|b| {
                    b.key == KeyCode::Char('q') && b.effect == Effect::Action(Action::Quit)
                }),
                "q not advertised in state {:?}",
                nav
            );
            assert_eq!(
                resolve(&ctx, KeyCode::Char('q')),
                Some(KeyResolution::Action(Action::Quit)),
                "q does not quit in state {:?}",
                nav
            );
        }
    }

    #[test]
    fn unadvertised_key_is_not_handled() {
        let idle = Question::idle();
        // 's' syncs in the Added fullscreen pane, but not in Detected fullscreen.
        let detected_fs = Navigation { focus: NavFocus::Detected, depth: Some(NavDepth::Fullscreen) };
        let ctx = make_ctx(detected_fs, &idle);
        assert!(!bindings_for(&ctx).iter().any(|b| b.key == KeyCode::Char('s')));
        assert_eq!(resolve(&ctx, KeyCode::Char('s')), None);

        let added_fs = Navigation { focus: NavFocus::Added, depth: Some(NavDepth::Fullscreen) };
        let ctx = make_ctx(added_fs, &idle);
        assert_eq!(
            resolve(&ctx, KeyCode::Char('s')),
            Some(KeyResolution::Action(Action::Sync))
        );
    }

    #[test]
    fn every_question_binding_is_handled_and_q_quits_from_modals() {
        let questions = vec![
            Question::add_device("dev".to_string(), "1.2.3.4".to_string()),
            Question::trust_key("dev".to_string()),
            Question::rename("dev".to_string(), "dev2".to_string()),
            Question::delete("dev".to_string()),
            Question::wipe(2),
            Question::setup_cd("~/.zshrc".to_string()),
            Question::add_temp_user("dev".to_string()),
        ];
        for q in &questions {
            let ctx = make_ctx(Navigation::overview(), q);
            for b in bindings_for(&ctx) {
                assert!(
                    resolve(&ctx, b.key).is_some(),
                    "advertised key {:?} of '{}' is not handled",
                    b.key,
                    q.def().title
                );
            }
        }
        // 'q' quits from every modal question except the text-input ones,
        // where it is typed into the name.
        for q in &questions {
            let ctx = make_ctx(Navigation::overview(), q);
            let resolution = resolve(&ctx, KeyCode::Char('q'));
            if matches!(q, Question::Rename(..) | Question::AddTempUser(..)) {
                assert_eq!(resolution, Some(KeyResolution::Internal), "q in '{}'", q.def().title);
            } else {
                assert_eq!(
                    resolution,
                    Some(KeyResolution::Action(Action::Quit)),
                    "q in '{}'",
                    q.def().title
                );
            }
        }
    }

    #[test]
    fn help_panel_only_handles_its_own_keys() {
        let idle = Question::idle();
        let ctx = KeyContext {
            navigation: Navigation::overview(),
            question: &idle,
            show_help: true,
            has_users: false,
            confirmed_user: false,
        };
        for b in bindings_for(&ctx) {
            assert!(resolve(&ctx, b.key).is_some(), "advertised {:?} not handled", b.key);
        }
        // 's' is not advertised while the help panel is open, so it does nothing.
        assert_eq!(resolve(&ctx, KeyCode::Char('s')), None);
        assert_eq!(resolve(&ctx, KeyCode::Char('q')), Some(KeyResolution::Internal));
    }
}
