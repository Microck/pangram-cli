use super::tests::ready_state;
use super::*;

#[test]
fn bracketed_paste_is_one_literal_composer_edit() {
    let mut state = ready_state();
    state.keymap = Keymap::Vim;
    let payload = "alpha\tbeta\n?j\u{1f98a}";

    let pasted = reduce(state, AppEvent::Paste(payload.to_owned()));

    assert_eq!(pasted.state.composer.value(), payload);
    assert_eq!(pasted.state.focus, Focus::Composer);
    assert!(pasted.state.overlay.is_none());
    assert!(!pasted.state.public_link);
    assert!(!pasted.state.manual_save);
    assert!(!pasted.state.analysis.submitting);
    assert!(pasted.effects.is_empty());
}

#[test]
fn bracketed_paste_only_edits_the_visible_focused_composer() {
    let base = ready_state();
    let mut states = Vec::new();

    let mut other_focus = base.clone();
    other_focus.focus = Focus::PublicLink;
    states.push(other_focus);

    let mut covered = base.clone();
    covered.overlay = Some(Overlay::Help);
    states.push(covered);

    let mut too_small = base;
    too_small.terminal = TerminalSize {
        columns: MIN_WIDTH - 1,
        rows: MIN_HEIGHT - 1,
    };
    states.push(too_small);

    for state in states {
        let original = state.clone();
        let pasted = reduce(state, AppEvent::Paste("must not edit".to_owned()));
        assert_eq!(pasted.state.composer.value(), original.composer.value());
        assert_eq!(pasted.state.focus, original.focus);
        assert_eq!(pasted.state.overlay, original.overlay);
        assert!(pasted.effects.is_empty());
    }
}
