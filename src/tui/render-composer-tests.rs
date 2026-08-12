use super::test_support::{draw, ready_state};
use crate::tui::model::{
    AppEvent, CredentialEntry, Focus, KeyInput, Overlay, Route, TextField, reduce,
};

#[test]
fn below_minimum_overlay_preserves_the_underlying_state() {
    let mut state = ready_state(79, 23);
    state.route = Route::Active;
    state.focus = Focus::ActiveList;
    state.composer = TextField::from_value("preserve me".to_owned());
    let route_before = state.route;
    let focus_before = state.focus;
    let composer_before = state.composer.value().to_owned();
    let screen = draw(79, 23, &state);

    assert!(screen.row(2).starts_with(" Active"));
    assert!(screen.row(4).contains("No unfinished analyses."));
    assert!(screen.text().contains("Terminal too small"));
    assert!(screen.text().contains("Resize to at least 80x24."));
    assert_eq!(state.route, route_before);
    assert_eq!(state.focus, focus_before);
    assert_eq!(state.composer.value(), composer_before);
}

#[test]
fn hostile_composer_control_sequences_never_reach_rendered_cells() {
    let mut state = ready_state(120, 40);
    state.composer = TextField::from_value("safe\u{1b}[31mowned\nnext".to_owned());
    let screen = draw(120, 40, &state);
    let text = screen.text();

    assert!(text.contains("safe\u{FFFD}[31mowned"));
    assert!(text.contains("next"));
    assert!(
        screen
            .cells
            .iter()
            .flatten()
            .all(|cell| !cell.contains('\u{1b}'))
    );
}

#[test]
fn focused_composer_scrolls_horizontally_to_the_visible_edit_position() {
    let mut state = ready_state(80, 24);
    state.focus = Focus::Composer;
    state.composer = TextField::from_value(format!("START_MARKER_{}_END_MARKER", "x".repeat(90)));

    let screen = draw(80, 24, &state);

    assert!(screen.text().contains("END_MARKER"));
    assert!(!screen.text().contains("START_MARKER"));
    let (cursor_x, cursor_y) = screen.cursor.expect("focused composer cursor");
    assert!((2..78).contains(&cursor_x));
    assert!((12..14).contains(&cursor_y));
}

#[test]
fn focused_composer_scrolls_vertically_and_loses_the_cursor_with_focus() {
    let mut state = ready_state(80, 24);
    state.focus = Focus::Composer;
    state.composer =
        TextField::from_value("FIRST_MARKER\nsecond line\nthird line\nLAST_MARKER".to_owned());

    let focused = draw(80, 24, &state);
    assert!(focused.text().contains("LAST_MARKER"));
    assert!(!focused.text().contains("FIRST_MARKER"));
    assert!(focused.cursor.is_some());

    state = reduce(state, AppEvent::Key(KeyInput::Tab)).state;
    assert_ne!(state.focus, Focus::Composer);
    assert!(draw(80, 24, &state).cursor.is_none());

    state.focus = Focus::Composer;
    state.overlay = Some(Overlay::Help);
    assert!(draw(80, 24, &state).cursor.is_none());
}

#[test]
fn credential_overlay_never_renders_cleartext() {
    let mut state = ready_state(120, 40);
    state.overlay = Some(Overlay::Credential(CredentialEntry::from_value(
        "pangram-secret-value".to_owned(),
    )));
    let text = draw(120, 40, &state).text();

    assert!(text.contains("API key: ******** (masked)"));
    assert!(!text.contains("pangram-secret-value"));
}
