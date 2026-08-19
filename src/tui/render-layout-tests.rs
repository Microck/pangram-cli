use super::test_support::{draw, ready_state};
use super::*;
use crate::tui::model::{ColorMode, CredentialEntry, Overlay};
use ratatui::style::{Color, Modifier};

#[test]
fn wide_layout_has_stable_rail_workspace_inspector_and_command_bar() {
    let mut state = ready_state(120, 40);
    state.focus = Focus::Quit;
    let screen = draw(120, 40, &state);

    assert!(screen.row(0)[..14].trim().is_empty());
    assert!(screen.row(0)[15..94].trim().is_empty());
    assert!(screen.row(0)[96..].trim().is_empty());
    assert!(screen.row(1).starts_with(" Pangram"));
    assert!(screen.row(1)[15..94].trim().is_empty());
    assert_eq!(screen.cells[0][14], "|");
    assert_eq!(screen.cells[0][94], " ");
    assert!(screen.row(1)[96..].starts_with(" Inspector"));
    assert!(screen.row(2)[15..94].trim().is_empty());
    assert!(screen.row(2)[96..].trim().is_empty());
    assert!(screen.row(11).contains("Ready to analyze"));
    assert!(screen.row(24).contains("AI detection"));
    assert!(screen.row(24).contains("Plagiarism"));
    assert!(screen.row(24).contains("Both"));
    assert!(screen.row(3).contains("Public link off"));
    assert!(screen.row(25)[15..94].trim().is_empty());
    assert!(screen.row(26).contains("Text"));
    assert!(screen.row(26).contains("Files"));
    assert!(screen.row(26).contains("unavailable"));
    assert!(screen.row(5).contains("Manual save off"));
    assert!(screen.row(27)[15..94].trim().is_empty());
    assert!(screen.row(28)[15..94].trim().is_empty());
    assert!(screen.row(29).contains("Text composer"));
    assert!(screen.row(8).contains("Words 0"));
    assert!(screen.cells[30][15..94].iter().any(|cell| cell != " "));
    assert!(screen.row(32).contains("Type or paste text here"));
    assert!(screen.row(10).contains("Estimate -"));
    assert!(screen.row(11)[96..].trim().is_empty());
    assert!(screen.row(12)[96..].trim().is_empty());
    assert!(screen.row(13).contains("Submit"));
    assert!(!screen.row(38)[15..94].contains('└'));
    assert!(screen.row(39).contains("enter  quit"));
    assert_eq!(screen.row(39).matches("enter").count(), 1);
    assert_eq!(screen.style(0, 39).background, Color::Rgb(28, 28, 28));
    assert_eq!(screen.cells[39][14], "|");
    assert_eq!(screen.style(14, 39).foreground, Color::DarkGray);
    assert_eq!(screen.style(15, 39).background, Color::Rgb(17, 17, 17));
    assert!(screen.row(39)[15..17].trim().is_empty());
    assert!(screen.row(39)[17..].starts_with(" tab "));
    assert!(!screen.text().contains('['));
}

#[test]
fn palette_uses_restrained_surfaces_and_one_primary_action() {
    let state = ready_state(120, 40);
    assert_eq!(state.route, Route::Analyze);
    assert_eq!(state.focus, Focus::Composer);
    let screen = draw(120, 40, &state);

    assert_eq!(screen.style(1, 1).foreground, Color::Rgb(255, 97, 6));
    assert_eq!(screen.style(1, 1).background, Color::Rgb(28, 28, 28));
    assert!(screen.style(1, 1).modifier.contains(Modifier::BOLD));
    assert_eq!(screen.style(15, 0).background, Color::Rgb(17, 17, 17));
    assert_eq!(screen.style(119, 20).background, Color::Rgb(17, 17, 17));
    assert_eq!(screen.style(15, 39).background, Color::Rgb(17, 17, 17));

    let selected_check = screen.row(24).find("AI detection").expect("selected check");
    assert_eq!(
        screen.style(selected_check, 24).background,
        Color::Rgb(255, 97, 6)
    );
    assert_eq!(
        screen.style(selected_check, 24).foreground,
        Color::Rgb(17, 17, 17)
    );
    assert_eq!(
        screen.style(selected_check - 1, 24).background,
        Color::Rgb(255, 97, 6)
    );
    assert_eq!(
        screen
            .style(selected_check + "AI detection".len(), 24)
            .background,
        Color::Rgb(255, 97, 6)
    );
    assert_eq!(
        screen.style(selected_check - 2, 24).background,
        Color::Rgb(17, 17, 17)
    );
    let inactive_check = screen.row(24).find("Plagiarism").expect("inactive check");
    assert_eq!(
        screen.style(inactive_check, 24).background,
        Color::Rgb(42, 42, 42)
    );
    let selected_route = screen.row(3).find("Analyze").expect("selected route");
    assert_eq!(
        screen.style(selected_route, 3).foreground,
        Color::Rgb(17, 17, 17)
    );
    assert_eq!(
        screen.style(selected_route, 3).background,
        Color::Rgb(255, 97, 6)
    );
    assert_eq!(
        screen.style(selected_route - 1, 3).background,
        Color::Rgb(255, 97, 6)
    );
    assert_eq!(
        screen.style(selected_route + "Analyze".len(), 3).background,
        Color::Rgb(255, 97, 6)
    );
    let inactive_route = screen.row(5).find("Active").expect("inactive route");
    assert_eq!(
        screen.style(inactive_route, 5).background,
        Color::Rgb(42, 42, 42)
    );
    assert_eq!(
        screen.style(inactive_route - 1, 5).background,
        Color::Rgb(42, 42, 42)
    );
    assert_eq!(
        screen.style(inactive_route + "Active".len(), 5).background,
        Color::Rgb(42, 42, 42)
    );
    assert_eq!(
        screen.style(inactive_route - 2, 5).background,
        Color::Rgb(28, 28, 28)
    );
    let mut route_focused = ready_state(120, 40);
    route_focused.focus = Focus::Routes;
    let route_focused_screen = draw(120, 40, &route_focused);
    let focused_route = route_focused_screen
        .row(3)
        .find(Route::Analyze.name())
        .expect("focused route");
    assert_eq!(route_focused_screen.cells[3][focused_route - 2], ">");
    assert_eq!(
        route_focused_screen.style(focused_route - 1, 3).background,
        Color::Rgb(255, 97, 6)
    );
    let composer_text = screen
        .row(32)
        .find("Type or paste text here")
        .expect("composer placeholder");
    assert_eq!(
        screen.style(composer_text, 32).background,
        Color::Rgb(28, 28, 28)
    );
    assert_eq!(screen.style(20, 20).background, Color::Rgb(17, 17, 17));
    let submit = screen.row(13).find("Submit").expect("submit action");
    assert_eq!(screen.style(submit, 13).background, Color::Rgb(255, 97, 6));
    assert_eq!(screen.style(submit, 13).foreground, Color::Rgb(17, 17, 17));
    assert_eq!(
        screen.style(submit - 1, 13).background,
        Color::Rgb(255, 97, 6)
    );
    assert_eq!(
        screen.style(submit + "Submit".len(), 13).background,
        Color::Rgb(255, 97, 6)
    );
    assert_eq!(
        screen.style(submit - 2, 13).background,
        Color::Rgb(17, 17, 17)
    );
    assert_eq!(
        muted_style(ColorMode::TrueColor).fg,
        Some(Color::Rgb(138, 138, 138))
    );
    assert_eq!(muted_style(ColorMode::Ansi).fg, Some(Color::Indexed(245)));
    assert_eq!(
        separator_style(ColorMode::TrueColor).fg,
        Some(Color::DarkGray)
    );

    let mut focused = ready_state(120, 40);
    focused.focus = Focus::Submit;
    let focused_screen = draw(120, 40, &focused);
    let focused_submit = focused_screen
        .row(13)
        .find("Submit")
        .expect("focused submit action");
    assert_eq!(
        focused_screen.style(focused_submit, 13).background,
        Color::Rgb(255, 97, 6)
    );
    assert_eq!(
        focused_screen.style(focused_submit, 13).foreground,
        Color::Rgb(17, 17, 17)
    );
    assert!(
        !focused_screen
            .style(focused_submit, 13)
            .modifier
            .contains(Modifier::UNDERLINED)
    );

    let mut ansi = ready_state(120, 40);
    ansi.color_mode = ColorMode::Ansi;
    let ansi_screen = draw(120, 40, &ansi);
    assert_eq!(ansi_screen.style(15, 0).background, Color::Indexed(233));
    assert_eq!(ansi_screen.style(1, 1).background, Color::Indexed(235));
}

#[test]
fn interactive_surfaces_never_render_terminal_underlining() {
    for color_mode in [ColorMode::TrueColor, ColorMode::Ansi, ColorMode::None] {
        let mut state = ready_state(120, 40);
        state.color_mode = color_mode;
        for focus in [Focus::Routes, Focus::CheckPlagiarism, Focus::Submit] {
            state.focus = focus;
            let screen = draw(120, 40, &state);
            assert!(
                screen
                    .styles
                    .iter()
                    .flatten()
                    .all(|style| !style.modifier.contains(Modifier::UNDERLINED))
            );
        }

        state.overlay = Some(Overlay::Credential(CredentialEntry::from_value(
            String::new(),
        )));
        let overlay = draw(120, 40, &state);
        assert!(
            overlay
                .styles
                .iter()
                .flatten()
                .all(|style| !style.modifier.contains(Modifier::UNDERLINED))
        );
    }
}

#[test]
fn no_color_keeps_every_state_legible_without_styling() {
    let mut state = ready_state(120, 40);
    state.color_mode = ColorMode::None;

    let screen = draw(120, 40, &state);

    assert_eq!(screen.style(1, 1).foreground, Color::Reset);
    assert_eq!(screen.style(1, 1).background, Color::Reset);
    assert!(screen.text().contains("* Analyze"));
    assert!(screen.text().contains("* AI detection"));
    assert!(screen.text().contains("> Text composer"));
}

#[test]
fn selected_check_and_its_conservative_estimate_render_together() {
    let mut state = ready_state(120, 40);
    state.text_mode = crate::analysis::TextAnalysisMode::Plagiarism;
    state.composer = crate::tui::model::TextField::from_value("one".to_owned());

    let screen = draw(120, 40, &state);

    assert!(screen.row(24).contains("AI detection"));
    assert!(screen.row(24).contains("Plagiarism"));
    assert!(screen.row(24).contains("Both"));
    assert!(screen.text().contains("Public link n/a"));
    assert!(screen.text().contains("Estimate 5 units"));
}

#[test]
fn empty_input_has_no_billable_estimate() {
    let screen = draw(120, 40, &ready_state(120, 40));

    assert!(screen.text().contains("  Words 0"));
    assert!(screen.text().contains("  Estimate -"));
    assert!(!screen.text().contains("Estimate 1 unit"));
}

#[test]
fn hundred_column_layout_uses_tabs_and_flows_settings_below_workspace() {
    let mut state = ready_state(100, 30);
    state.route = Route::Settings;
    state.focus = Focus::SettingsKeymap;
    let screen = draw(100, 30, &state);

    for route in Route::ALL {
        assert!(
            screen.row(0).contains(route.name()),
            "missing route {}",
            route.name()
        );
    }
    assert!(screen.row(1).trim().is_empty());
    assert!(screen.row(2).starts_with("  Account"));
    assert!(screen.row(3).trim().is_empty());
    assert!(screen.row(4).contains("Authentication"));
    assert!(screen.row(4).contains("configured"));
    assert!(screen.row(5).trim().is_empty());
    assert!(screen.row(6).trim().is_empty());
    assert!(screen.row(7).starts_with("  Preferences"));
    assert!(screen.row(8).trim().is_empty());
    assert!(screen.row(9).contains("History"));
    assert!(screen.row(9).contains("disabled"));
    assert!(
        screen.row(13).contains("Keymap"),
        "unexpected keymap row: {:?}",
        screen.row(13)
    );
    assert!(screen.row(13).contains("Regular"));
    assert!(screen.row(15).contains("Motion"));
    assert!(screen.row(15).contains("full"));
    assert!(screen.row(17).contains("Updates"));
    assert!(screen.row(17).contains("disabled"));
    assert!(screen.row(20).starts_with("  Diagnostics"));
    assert!(screen.row(22).contains("Run `pangram doctor`"));
    assert_eq!(screen.text().matches("Settings").count(), 1);
    assert!(!screen.text().contains("Configuration"));
    assert!(screen.row(27).trim().is_empty());
    assert!(screen.row(28).contains("enter  change"));
    assert!(screen.row(29).trim().is_empty());
    let contextual_key = screen.row(28).rfind("enter").expect("contextual key");
    assert_eq!(
        screen.style(contextual_key, 28).background,
        Color::Rgb(255, 97, 6)
    );
}

#[test]
fn narrow_analyze_uses_terminal_line_height_between_controls_and_rules() {
    let screen = draw(100, 30, &ready_state(100, 30));

    assert!(screen.row(0).contains("pangram"));
    assert!(screen.row(6).contains("Ready to analyze"));
    assert!(screen.row(14).contains("Check"));
    assert!(screen.row(15).trim().is_empty());
    assert!(screen.row(16).contains("Input"));
    assert!(screen.row(17).trim().is_empty());
    assert!(screen.row(18).trim().is_empty());
    assert!(screen.row(19).contains("Text composer"));
    assert!(!screen.row(20).trim().is_empty());
    assert!(screen.row(22).contains("Type or paste text here"));

    assert!(screen.row(27).contains("Public link off"));
    assert!(screen.row(27).contains("Manual save off"));
    assert!(screen.row(27).contains("0 words | Estimate -"));
    assert!(screen.row(27).contains("Submit"));
    assert!(screen.row(28).trim().is_empty());
    assert!(screen.row(29)[2..].starts_with(" tab "));
}

#[test]
fn minimum_layout_keeps_local_history_and_inspector_in_the_center_flow() {
    let mut state = ready_state(80, 24);
    state.route = Route::History;
    state.focus = Focus::HistorySearch;
    let screen = draw(80, 24, &state);

    assert!(screen.row(0).contains("History"));
    assert!(screen.row(1).trim().is_empty());
    assert!(screen.row(2).contains("> Search  empty"));
    assert!(!screen.row(2).contains("Local Pangram CLI history"));
    assert!(screen.row(18).starts_with("  Local history - Showing 0"));
    assert!(screen.text().contains("Selected: none"));
    assert!(screen.row(23).contains("enter  search"));
    assert!(!screen.row(23).contains("enter  quit"));
}

#[test]
fn narrow_route_tabs_keep_later_labels_in_fixed_columns() {
    let mut state = ready_state(100, 30);
    let analyze_screen = draw(100, 30, &state);
    let analyze_row = analyze_screen.row(0);
    let analyze_column = analyze_row.find("Analyze").expect("selected route");
    assert_eq!(
        analyze_screen.style(analyze_column, 0).foreground,
        Color::Rgb(17, 17, 17)
    );
    assert_eq!(
        analyze_screen.style(analyze_column, 0).background,
        Color::Rgb(255, 97, 6)
    );
    assert!(
        !analyze_screen
            .style(analyze_column, 0)
            .modifier
            .contains(Modifier::UNDERLINED)
    );
    for route in Route::ALL.into_iter().skip(1) {
        let label = route.name();
        let column = analyze_row.find(label).expect("inactive route");
        assert_eq!(
            analyze_screen.style(column, 0).background,
            Color::Rgb(42, 42, 42)
        );
        assert_eq!(
            analyze_screen.style(column - 1, 0).background,
            Color::Rgb(42, 42, 42)
        );
        assert_eq!(
            analyze_screen.style(column + label.len(), 0).background,
            Color::Rgb(42, 42, 42)
        );
        assert_eq!(
            analyze_screen.style(column - 2, 0).background,
            Color::Rgb(28, 28, 28)
        );
    }
    for routes in Route::ALL.windows(2) {
        let current = routes[0].name();
        let next = routes[1].name();
        assert_eq!(
            analyze_row.find(next).expect("next route"),
            analyze_row.find(current).expect("current route") + current.len() + 3,
            "unexpected gap between {current} and {next}"
        );
    }

    state.focus = Focus::Routes;
    let focused_screen = draw(100, 30, &state);
    assert_eq!(
        focused_screen.style(analyze_column, 0).background,
        Color::Rgb(255, 97, 6)
    );
    assert!(
        !focused_screen
            .style(analyze_column, 0)
            .modifier
            .contains(Modifier::UNDERLINED)
    );

    state.focus = Focus::Composer;
    state.route = Route::History;
    let history_screen = draw(100, 30, &state);
    let history_row = history_screen.row(0);
    let history_column = history_row.find("History").expect("selected route");
    assert_eq!(
        history_screen.style(history_column, 0).foreground,
        Color::Rgb(17, 17, 17)
    );
    assert_eq!(
        history_screen.style(history_column, 0).background,
        Color::Rgb(255, 97, 6)
    );

    for route in Route::ALL {
        let label = route.name();
        assert_eq!(
            analyze_row.find(label),
            history_row.find(label),
            "moved {label}"
        );
    }
}

#[test]
fn empty_active_centers_one_real_orange_analyze_action() {
    let mut state = ready_state(100, 30);
    state.route = Route::Active;
    state.focus = Focus::ActiveList;

    let screen = draw(100, 30, &state);
    let text = screen.text();
    let action_row = screen
        .cells
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, row)| row.concat().contains("Analyze").then_some(index))
        .expect("centered Analyze action");
    let action_column = screen
        .row(action_row)
        .rfind("Analyze")
        .expect("action label");

    assert!(text.contains("Nothing running"));
    assert!(text.contains("Start a new analysis when you're ready."));
    assert!(
        action_row > 8 && action_row < 22,
        "action row: {action_row}"
    );
    assert_eq!(screen.cells[action_row][action_column - 2], ">");
    assert_eq!(
        screen.style(action_column - 1, action_row).background,
        Color::Rgb(255, 97, 6)
    );
    assert_eq!(
        screen
            .style(action_column + "Analyze".len(), action_row)
            .background,
        Color::Rgb(255, 97, 6)
    );
}

#[test]
fn history_filters_and_context_actions_render_as_compact_targets() {
    let mut state = ready_state(100, 30);
    state.route = Route::History;
    state.focus = Focus::HistoryStatusFilter;

    let screen = draw(100, 30, &state);
    let search_row = screen
        .cells
        .iter()
        .position(|row| row.concat().contains("Search  empty"))
        .expect("search target");
    let search_column = screen.row(search_row).find("Search").expect("search label");
    let status_row = screen
        .cells
        .iter()
        .position(|row| row.concat().contains("Status  all"))
        .expect("status target");
    let status_column = screen.row(status_row).find("Status").expect("status label");
    let actions_row = screen
        .cells
        .iter()
        .position(|row| row.concat().contains("Rerun") && row.concat().contains("Export"))
        .expect("context actions");
    let rerun_column = screen.row(actions_row).find("Rerun").expect("rerun action");

    assert_eq!(
        screen.style(search_column - 1, search_row).background,
        Color::Rgb(42, 42, 42)
    );
    assert_eq!(screen.cells[status_row][status_column - 2], ">");
    assert_eq!(
        screen.style(status_column - 1, status_row).background,
        Color::Rgb(255, 97, 6)
    );
    assert_eq!(
        screen.style(rerun_column - 1, actions_row).background,
        Color::Rgb(42, 42, 42)
    );
    assert!(screen.text().contains("No saved analyses"));
}

#[test]
fn settings_use_plain_aligned_rows_with_one_focused_value_target() {
    let mut state = ready_state(100, 30);
    state.route = Route::Settings;
    state.focus = Focus::SettingsHistory;

    let screen = draw(100, 30, &state);
    let auth_row = screen
        .cells
        .iter()
        .position(|row| {
            let line = row.concat();
            line.contains("Authentication") && line.contains("configured")
        })
        .expect("authentication target");
    let auth_column = screen
        .row(auth_row)
        .find("Authentication")
        .expect("authentication");
    let history_row = screen
        .cells
        .iter()
        .position(|row| {
            let line = row.concat();
            line.contains("History") && line.contains("disabled")
        })
        .expect("history target");
    let history_column = screen.row(history_row).find("History").expect("history");

    let auth_value = screen
        .row(auth_row)
        .find("configured")
        .expect("authentication value");
    let history_value = screen
        .row(history_row)
        .find("disabled")
        .expect("history value");

    assert_eq!(auth_value, history_value);
    assert_eq!(
        screen.style(auth_column, auth_row).background,
        Color::Rgb(17, 17, 17)
    );
    assert_eq!(
        screen.style(auth_value, auth_row).background,
        Color::Rgb(17, 17, 17)
    );
    assert_eq!(screen.cells[history_row][history_column - 2], ">");
    assert_eq!(
        screen.style(history_column, history_row).background,
        Color::Rgb(17, 17, 17)
    );
    assert_eq!(
        screen.style(history_value - 1, history_row).background,
        Color::Rgb(255, 97, 6)
    );
    assert_eq!(
        screen
            .style(history_value + "disabled".len(), history_row)
            .background,
        Color::Rgb(255, 97, 6)
    );
}
