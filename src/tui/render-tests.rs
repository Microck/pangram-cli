use std::str::FromStr;

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::*;
use crate::analysis::AnalysisProgress;
use crate::domain::{
    AiClassification, AiDetectionResult, Analysis, AnalysisId, AnalysisInput, AnalysisInputKind,
    AnalysisStatus, AnalysisSummary, Check, CheckKind, CheckState, Confidence, FileInput, Fraction,
    NonEmptyString, OrderedChecks, Percentage, PlagiarismMatch, PlagiarismResult, Provenance,
    Provider, SaveState, Segment, Sha256Hash, SubmissionOutcome, TextInput, TextOrigin,
    UpstreamTaskId, UpstreamTaskIds, UtcTimestamp,
};
use crate::history::HistoryExportFormat;
use crate::output::{CanonicalError, ErrorCode};
use crate::tui::history::{ExportRequest, RedactedAnalysis, SelectionMove};
use crate::tui::model::{
    AnalysisFailure, AppEvent, CredentialEntry, HistoryExportField, KeyInput, SettingsDraft,
    StartupState, TerminalSize, TextField, reduce,
};

const FIXED_ANALYSIS_ID: &str = "anl_01983c20-0180-7a80-a001-000000000501";
const FIXED_TIMESTAMP: &str = "2026-08-09T12:00:00Z";

struct Screen {
    cells: Vec<Vec<String>>,
}

impl Screen {
    fn row(&self, y: usize) -> String {
        self.cells[y].concat()
    }

    fn text(&self) -> String {
        self.cells
            .iter()
            .map(|row| row.concat())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn ready_state(width: u16, height: u16) -> AppState {
    AppState::new(
        TerminalSize {
            columns: width,
            rows: height,
        },
        StartupState {
            settings: SettingsDraft {
                credential_present: true,
                update_preference: Some(false),
                ..SettingsDraft::default()
            },
            ..StartupState::default()
        },
    )
}

fn draw(width: u16, height: u16, state: &AppState) -> Screen {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("create test terminal");
    terminal
        .draw(|frame| render(frame, state))
        .expect("render TUI frame");
    let buffer = terminal.backend().buffer();
    let cells = (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect()
        })
        .collect();
    Screen { cells }
}

fn analysis_id() -> AnalysisId {
    AnalysisId::from_str(FIXED_ANALYSIS_ID).expect("fixed analysis ID")
}

fn timestamp() -> UtcTimestamp {
    UtcTimestamp::from_str(FIXED_TIMESTAMP).expect("fixed timestamp")
}

fn canonical_error(code: ErrorCode, message: &str) -> CanonicalError {
    CanonicalError::new(code, message).expect("valid canonical error")
}

fn terminal_analysis(
    save_state: SaveState,
    dashboard_link: Option<String>,
) -> Analysis<CanonicalError> {
    let result = AiDetectionResult {
        classification: AiClassification::Human,
        headline: "Human-written".to_owned(),
        prediction: "The text appears human-written.".to_owned(),
        fraction_ai: Fraction::new(0.0).expect("valid fraction"),
        fraction_ai_assisted: Fraction::new(0.0).expect("valid fraction"),
        fraction_human: Fraction::new(1.0).expect("valid fraction"),
        num_ai_segments: 0,
        num_ai_assisted_segments: 0,
        num_human_segments: 0,
        segments: Vec::new(),
        dashboard_link,
    };
    let check: CheckState<AiDetectionResult, CanonicalError> = CheckState::Succeeded {
        upstream: None,
        result,
    };
    analysis_with_check(Check::AiDetection(check), save_state)
}

fn queued_analysis(id: AnalysisId) -> Analysis<CanonicalError> {
    let input = TextInput::new(
        TextOrigin::Literal,
        None,
        Sha256Hash::digest(b"A queued analysis fixture."),
        26,
        4,
        None,
    )
    .expect("valid text input");
    let task_id = UpstreamTaskId::new(format!("task-{id}")).expect("valid task ID");
    let checks = OrderedChecks::new([Check::AiDetection(CheckState::Queued { upstream: None })])
        .expect("one check");
    Analysis::new(
        id,
        SubmissionOutcome::Accepted,
        AnalysisInput::Text(input),
        checks,
        SaveState::Ephemeral,
        Provenance {
            provider: Provider::Pangram,
            upstream_version: None,
            upstream_task_ids: Some(UpstreamTaskIds::new(vec![task_id]).expect("one task ID")),
            upstream_bulk_id: None,
            submitted_at: Some(timestamp()),
            completed_at: None,
        },
        None,
        None,
        timestamp(),
        timestamp(),
        None,
    )
    .expect("valid queued analysis")
}

fn failed_terminal_analysis() -> Analysis<CanonicalError> {
    let check: CheckState<AiDetectionResult, CanonicalError> = CheckState::Failed {
        upstream: None,
        error: canonical_error(
            ErrorCode::UpstreamAnalysisFailed,
            "Pangram could not complete this analysis.",
        ),
    };
    analysis_with_check(Check::AiDetection(check), SaveState::Ephemeral)
}

fn partial_terminal_analysis() -> Analysis<CanonicalError> {
    let checks = OrderedChecks::new([
        Check::AiDetection(CheckState::Failed {
            upstream: None,
            error: canonical_error(
                ErrorCode::UpstreamAnalysisFailed,
                "AI check failed.\u{1b}[31m\nRetry later.",
            ),
        }),
        Check::Plagiarism(CheckState::Succeeded {
            upstream: None,
            result: PlagiarismResult {
                plagiarism_detected: true,
                total_sentences: 2,
                plagiarized_sentence_count: 1,
                percent_plagiarized: Percentage::new(50.0).expect("valid percentage"),
                matches: vec![PlagiarismMatch {
                    source_url: "https://e.test/\u{1b}[32m\nx".to_owned(),
                    matched_text: "evidence\u{1b}[2J\nshown".to_owned(),
                    similarity_score: Fraction::new(0.91).expect("valid fraction"),
                }],
            },
        }),
    ])
    .expect("canonical partial checks");
    let input = TextInput::new(
        TextOrigin::Literal,
        None,
        Sha256Hash::digest(b"A deterministic partial-result test sentence."),
        45,
        5,
        None,
    )
    .expect("valid text input");
    analysis_with_input(
        analysis_id(),
        AnalysisInput::Text(input),
        checks,
        SaveState::Ephemeral,
    )
}

fn analysis_with_check(
    check: Check<CanonicalError>,
    save_state: SaveState,
) -> Analysis<CanonicalError> {
    let input = TextInput::new(
        TextOrigin::Literal,
        None,
        Sha256Hash::digest(b"A deterministic test sentence."),
        30,
        4,
        None,
    )
    .expect("valid text input");
    analysis_with_input(
        analysis_id(),
        AnalysisInput::Text(input),
        OrderedChecks::new([check]).expect("one check is valid"),
        save_state,
    )
}

fn analysis_with_input(
    id: AnalysisId,
    input: AnalysisInput,
    checks: OrderedChecks<Check<CanonicalError>>,
    save_state: SaveState,
) -> Analysis<CanonicalError> {
    Analysis::new(
        id,
        SubmissionOutcome::Terminal,
        input,
        checks,
        save_state,
        Provenance {
            provider: Provider::Pangram,
            upstream_version: Some("4.0".to_owned()),
            upstream_task_ids: None,
            upstream_bulk_id: None,
            submitted_at: None,
            completed_at: None,
        },
        None,
        None,
        timestamp(),
        timestamp(),
        Some(timestamp()),
    )
    .expect("valid terminal analysis")
}

fn history_id(index: u8) -> AnalysisId {
    format!("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a{index:02x}")
        .parse()
        .expect("canonical history fixture ID")
}

fn history_summary(index: u8, save_state: SaveState, display_name: &str) -> AnalysisSummary {
    history_summary_with_status(index, AnalysisStatus::Succeeded, save_state, display_name)
}

fn history_summary_with_status(
    index: u8,
    status: AnalysisStatus,
    save_state: SaveState,
    display_name: &str,
) -> AnalysisSummary {
    AnalysisSummary {
        id: history_id(index),
        status,
        checks: OrderedChecks::new([CheckKind::AiDetection]).expect("one check"),
        save_state,
        input_kind: AnalysisInputKind::Text,
        display_name: Some(display_name.to_owned()),
        created_at: timestamp(),
    }
}

fn history_state(width: u16, height: u16) -> AppState {
    let mut state = ready_state(width, height);
    state.route = Route::History;
    state.history = Default::default();
    state
}

fn hostile_history_analysis() -> Analysis<CanonicalError> {
    let ai_result = AiDetectionResult {
        classification: AiClassification::Mixed,
        headline: "headline\u{1b}[31mforged\nnext".to_owned(),
        prediction: "prediction\u{1b}[2Jcleared".to_owned(),
        fraction_ai: Fraction::new(0.4).expect("valid fraction"),
        fraction_ai_assisted: Fraction::new(0.2).expect("valid fraction"),
        fraction_human: Fraction::new(0.4).expect("valid fraction"),
        num_ai_segments: 8,
        num_ai_assisted_segments: 0,
        num_human_segments: 0,
        segments: (1..=8)
            .map(|index| Segment {
                text: if index == 1 {
                    "SECRET_SEGMENT_TEXT".to_owned()
                } else {
                    format!("segment evidence {index}")
                },
                label: NonEmptyString::new(if index == 1 {
                    "segment\u{1b}[32mlabel".to_owned()
                } else {
                    format!("segment-{index}")
                })
                .expect("segment label"),
                ai_assistance_score: Fraction::new(0.4).expect("valid fraction"),
                confidence: Confidence::High,
                start_index: (index - 1) * 10,
                end_index: index * 10,
                word_count: 1,
                token_length: 1,
                humanizer_score: Fraction::new(0.0).expect("valid fraction"),
                is_humanized: false,
            })
            .collect(),
        dashboard_link: Some("https://example.test/result\u{1b}[0mreset".to_owned()),
    };
    let plagiarism_result = PlagiarismResult {
        plagiarism_detected: true,
        total_sentences: 8,
        plagiarized_sentence_count: 8,
        percent_plagiarized: Percentage::new(100.0).expect("valid percentage"),
        matches: (1..=8)
            .map(|index| PlagiarismMatch {
                source_url: if index == 1 {
                    "https://source.test/\u{1b}[31murl".to_owned()
                } else {
                    format!("https://source.test/{index}")
                },
                matched_text: if index == 1 {
                    "matched\u{1b}[2Jtext".to_owned()
                } else {
                    format!("match evidence {index}")
                },
                similarity_score: Fraction::new(0.9).expect("valid fraction"),
            })
            .collect(),
    };
    let checks = OrderedChecks::new([
        Check::AiDetection(CheckState::Succeeded {
            upstream: None,
            result: ai_result,
        }),
        Check::Plagiarism(CheckState::Succeeded {
            upstream: None,
            result: plagiarism_result,
        }),
    ])
    .expect("ordered checks");
    let input = FileInput {
        filename: NonEmptyString::new("report\u{1b}[31m.txt").expect("filename"),
        media_type: NonEmptyString::new("text/plain\u{1b}[0m").expect("media type"),
        sha256: Sha256Hash::digest(b"SECRET_RETAINED_FILE"),
        size_bytes: 20,
        path: Some("/secret/original/path.txt".to_owned()),
        extracted_text: Some("SECRET_EXTRACTED_TEXT".to_owned()),
    };
    analysis_with_input(
        history_id(1),
        AnalysisInput::File(input),
        checks,
        SaveState::SavedHistory,
    )
}

#[test]
fn wide_layout_has_stable_rail_workspace_inspector_and_command_bar() {
    let mut state = ready_state(120, 40);
    state.focus = Focus::Quit;
    let screen = draw(120, 40, &state);

    assert!(screen.row(1).starts_with(" Pangram"));
    assert!(screen.row(0)[18..].starts_with(" Analyze"));
    assert_eq!(screen.cells[0][17], "|");
    assert_eq!(screen.cells[0][89], "|");
    assert!(screen.row(1)[90..].starts_with(" Inspector"));
    assert!(screen.row(2).contains("[x] AI detection - available"));
    assert!(screen.row(3).contains("[ ] Plagiarism - unavailable"));
    assert!(screen.row(6).contains("[x] Text - available"));
    assert!(screen.row(7).contains("[ ] Files - unavailable"));
    assert!(screen.row(2).contains("Public link: off"));
    assert!(screen.row(3).contains("Manual save: off"));
    assert!(screen.row(39).contains("> [Enter] Quit <"));
}

#[test]
fn hundred_column_layout_uses_tabs_and_flows_settings_below_workspace() {
    let mut state = ready_state(100, 30);
    state.route = Route::Settings;
    state.focus = Focus::SettingsKeymap;
    let screen = draw(100, 30, &state);

    for route in ["Analyze", "Active", "History", "[Settings]"] {
        assert!(screen.row(0).contains(route), "missing route {route}");
    }
    assert!(screen.row(2).starts_with(" Settings"));
    assert!(screen.row(4).contains("Authentication: configured"));
    assert!(screen.row(8).contains("Motion: full"));
    assert!(screen.row(22).starts_with(" Configuration"));
    assert!(screen.row(29).contains("[Enter] Quit"));
}

#[test]
fn minimum_layout_keeps_local_history_and_inspector_in_the_center_flow() {
    let mut state = ready_state(80, 24);
    state.route = Route::History;
    state.focus = Focus::HistorySearch;
    let screen = draw(80, 24, &state);

    assert!(screen.row(0).contains("[History]"));
    assert!(
        screen
            .row(2)
            .starts_with(" History - Local Pangram CLI history")
    );
    assert!(screen.row(3).contains("> Search literal: [empty]"));
    assert!(screen.row(16).starts_with(" Local history - Showing 0"));
    assert!(screen.row(23).contains("[/] Search"));
    assert!(screen.row(23).contains("[Enter] Quit"));
}

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
fn credential_overlay_never_renders_cleartext() {
    let mut state = ready_state(120, 40);
    state.overlay = Some(Overlay::Credential(CredentialEntry::from_value(
        "pangram-secret-value".to_owned(),
    )));
    let text = draw(120, 40, &state).text();

    assert!(text.contains("API key: ******** (masked)"));
    assert!(!text.contains("pangram-secret-value"));
}

#[test]
fn submitting_state_distinguishes_the_one_time_request_from_polling() {
    let mut state = ready_state(120, 40);
    state.analysis.submitting = true;
    let text = draw(120, 40, &state).text();

    assert!(text.contains("Submitting analysis"));
    assert!(text.contains("The request is being sent once."));
    assert!(!text.contains("Analysis in progress"));
}

#[test]
fn progress_state_preserves_the_canonical_identity_and_upstream_stage() {
    let accepted = reduce(
        ready_state(120, 40),
        AppEvent::AnalysisAccepted(queued_analysis(analysis_id())),
    );
    let progress = AnalysisProgress {
        analysis_id: analysis_id(),
        task_id: UpstreamTaskId::new("task-render-progress").expect("valid task ID"),
        last_stage: NonEmptyString::new("DETECTING_AI").expect("valid stage"),
    };
    let mut state = reduce(accepted.state, AppEvent::AnalysisProgress(progress)).state;
    let text = draw(120, 40, &state).text();

    assert!(text.contains("Analysis in progress"));
    assert!(text.contains("Stage: DETECTING_AI"));
    assert!(!text.contains("Submitting analysis"));

    state.route = Route::Active;
    state.focus = Focus::ActiveList;
    let active_text = draw(120, 40, &state).text();
    assert!(active_text.contains(&format!("{FIXED_ANALYSIS_ID} - running - this session")));
}

#[test]
fn active_combines_session_work_with_saved_unfinished_history() {
    let state = ready_state(120, 40);
    let request = state.history.load_request();
    let loaded = reduce(
        state,
        AppEvent::HistoryLoaded {
            request,
            result: Ok(vec![
                history_summary_with_status(
                    1,
                    AnalysisStatus::Queued,
                    SaveState::SavedManual,
                    "queued record",
                ),
                history_summary_with_status(
                    2,
                    AnalysisStatus::Running,
                    SaveState::SavedHistory,
                    "running record",
                ),
                history_summary(3, SaveState::SavedHistory, "finished record"),
            ]),
        },
    );
    let mut state = reduce(
        loaded.state,
        AppEvent::AnalysisAccepted(queued_analysis(analysis_id())),
    )
    .state;
    state.route = Route::Active;
    state.focus = Focus::ActiveList;

    let text = draw(120, 40, &state).text();

    assert!(text.contains(&format!("> {FIXED_ANALYSIS_ID} - queued - this session")));
    assert!(text.contains(&format!("{} - queued - saved history", history_id(1))));
    assert!(text.contains(&format!("{} - running - saved history", history_id(2))));
    assert!(!text.contains(&history_id(3).to_string()));
}

#[test]
fn active_selection_reaches_rows_beyond_the_first_viewport() {
    let state = ready_state(120, 40);
    let request = state.history.load_request();
    let summaries = (1..=8)
        .map(|index| {
            history_summary_with_status(
                index,
                AnalysisStatus::Queued,
                SaveState::SavedHistory,
                "queued record",
            )
        })
        .collect();
    let mut state = reduce(
        state,
        AppEvent::HistoryLoaded {
            request,
            result: Ok(summaries),
        },
    )
    .state;
    state.route = Route::Active;
    state.focus = Focus::ActiveList;

    state = reduce(state, AppEvent::Key(KeyInput::End)).state;
    let text = draw(120, 40, &state).text();

    assert!(text.contains(&format!("> {} - queued - saved history", history_id(8))));
    assert!(!text.contains(&history_id(1).to_string()));
}

#[test]
fn progress_and_terminal_events_change_only_the_matching_active_identity() {
    let other_id = history_id(2);
    let first = reduce(
        ready_state(120, 40),
        AppEvent::AnalysisAccepted(queued_analysis(analysis_id())),
    );
    let accepted = reduce(
        first.state,
        AppEvent::AnalysisAccepted(queued_analysis(other_id)),
    );
    let progressed = reduce(
        accepted.state,
        AppEvent::AnalysisProgress(AnalysisProgress {
            analysis_id: analysis_id(),
            task_id: UpstreamTaskId::new("task-progress-owner").expect("valid task ID"),
            last_stage: NonEmptyString::new("DETECTING_AI").expect("valid stage"),
        }),
    );
    assert_eq!(
        progressed.state.active.status(analysis_id()),
        Some(AnalysisStatus::Running)
    );
    assert_eq!(
        progressed.state.active.status(other_id),
        Some(AnalysisStatus::Queued)
    );

    let stale = reduce(
        progressed.state,
        AppEvent::AnalysisProgress(AnalysisProgress {
            analysis_id: history_id(9),
            task_id: UpstreamTaskId::new("task-stale").expect("valid task ID"),
            last_stage: NonEmptyString::new("STALE").expect("valid stage"),
        }),
    );
    assert_eq!(
        stale.state.active.status(analysis_id()),
        Some(AnalysisStatus::Running)
    );
    assert_eq!(
        stale.state.active.status(other_id),
        Some(AnalysisStatus::Queued)
    );
    assert_eq!(
        stale
            .state
            .analysis
            .progress
            .as_ref()
            .map(|progress| progress.analysis_id),
        Some(analysis_id())
    );

    let completed = reduce(
        stale.state,
        AppEvent::AnalysisFinished(terminal_analysis(SaveState::Ephemeral, None)),
    );
    assert_eq!(completed.state.active.status(analysis_id()), None);
    assert_eq!(
        completed.state.active.status(other_id),
        Some(AnalysisStatus::Queued)
    );
}

#[test]
fn succeeded_terminal_result_renders_classification_and_sanitized_dashboard_link() {
    let unsafe_link = "https://dashboard.example/result\u{1b}[31mforged\nsecond-line";
    let analysis = terminal_analysis(SaveState::Ephemeral, Some(unsafe_link.to_owned()));
    let state = reduce(ready_state(120, 40), AppEvent::AnalysisFinished(analysis)).state;
    let text = draw(120, 40, &state).text();

    assert!(text.contains("Overall: succeeded"));
    assert!(text.contains("Classification: Human"));
    assert!(text.contains("AI 0.0% | AI-assisted 0.0% | Human 100.0%"));
    assert!(text.contains("Public dashboard: https://dashboard.example/result [31mforged"));
    assert!(text.contains("second-line"));
    assert!(!text.contains('\u{1b}'));
}

#[test]
fn failed_submission_and_terminal_check_failures_have_distinct_results() {
    let submission_failure = AnalysisFailure {
        analysis_id: analysis_id(),
        error: canonical_error(
            ErrorCode::NetworkUnavailable,
            "The Pangram service is unavailable.",
        ),
    };
    let failed_state = reduce(
        ready_state(120, 40),
        AppEvent::AnalysisFailed(submission_failure),
    )
    .state;
    let failed_text = draw(120, 40, &failed_state).text();

    assert!(failed_text.contains("Analysis failed"));
    assert!(failed_text.contains(FIXED_ANALYSIS_ID));
    assert!(failed_text.contains("Error: The Pangram service is unavailable."));

    let terminal_state = reduce(
        ready_state(120, 40),
        AppEvent::AnalysisFinished(failed_terminal_analysis()),
    )
    .state;
    let terminal_text = draw(120, 40, &terminal_state).text();

    assert!(terminal_text.contains("Overall: failed"));
    assert!(
        terminal_text.contains("AI detection failed: Pangram could not complete this analysis.")
    );
}

#[test]
fn failed_history_rerun_leaves_active_and_renders_the_terminal_error() {
    let mut state = history_state(120, 40);
    state.history.reload(vec![history_summary(
        1,
        SaveState::SavedHistory,
        "rerun source",
    )]);
    state.focus = Focus::HistoryRerun;

    let requested = reduce(state, AppEvent::Key(KeyInput::Enter));
    let prepared = reduce(
        requested.state,
        AppEvent::HistoryRerunPrepared {
            analysis_id: history_id(1),
            result: Ok(analysis_id()),
        },
    );
    assert_eq!(prepared.state.route, Route::Active);

    let failed = reduce(
        prepared.state,
        AppEvent::AnalysisFailed(AnalysisFailure {
            analysis_id: analysis_id(),
            error: canonical_error(
                ErrorCode::NetworkUnavailable,
                "The Pangram service is unavailable.",
            ),
        }),
    )
    .state;
    let text = draw(120, 40, &failed).text();

    assert_eq!(failed.route, Route::Analyze);
    assert_eq!(failed.focus, Focus::Submit);
    assert!(text.contains("Analysis failed"));
    assert!(text.contains("Error: The Pangram service is unavailable."));
    assert!(!text.contains("Rerun started."));
}

#[test]
fn partial_terminal_result_keeps_succeeded_evidence_and_failed_diagnostic_terminal_safe() {
    let state = reduce(
        ready_state(120, 40),
        AppEvent::AnalysisFinished(partial_terminal_analysis()),
    )
    .state;
    let text = draw(120, 40, &state).text();

    assert!(text.contains("Overall: partial"));
    assert!(text.contains("Plagiarism: detected - 50.0% across 1/2 sentences"));
    assert!(text.contains("Match 1: 91.0% - https://e.test/ [32m x - evidence [2J shown"));
    assert!(text.contains("AI detection failed: AI check failed. [31m Retry later."));
    assert!(!text.contains('\u{1b}'));
}

#[test]
fn terminal_result_labels_each_canonical_save_state() {
    for (save_state, expected) in [
        (SaveState::Ephemeral, "Save state: ephemeral"),
        (SaveState::SavedManual, "Save state: saved manual"),
        (SaveState::SavedHistory, "Save state: saved history"),
    ] {
        let analysis = terminal_analysis(save_state, None);
        let state = reduce(ready_state(120, 40), AppEvent::AnalysisFinished(analysis)).state;
        let text = draw(120, 40, &state).text();

        assert!(text.contains(expected), "missing {expected}");
    }
}

#[test]
fn save_failure_notice_is_visible_and_terminal_safe() {
    let state = reduce(
        ready_state(120, 40),
        AppEvent::Notice("History write failed.\u{1b}[31m\nTry again.".to_owned()),
    )
    .state;
    let text = draw(120, 40, &state).text();

    assert!(text.contains("Notice: History write"));
    assert!(text.contains("failed. [31m Try again."));
    assert!(!text.contains('\u{1b}'));
}

#[test]
fn wide_history_shows_six_complete_summaries_and_every_save_state() {
    let mut state = history_state(120, 40);
    state.focus = Focus::HistoryList;
    state.history.reload(vec![
        history_summary(0, SaveState::Ephemeral, "ephemeral record"),
        history_summary(1, SaveState::SavedManual, "manual record"),
        history_summary(2, SaveState::SavedHistory, "history record"),
        history_summary(3, SaveState::SavedHistory, "fourth record"),
        history_summary(4, SaveState::SavedHistory, "fifth record"),
        history_summary(5, SaveState::SavedHistory, "sixth record"),
    ]);

    let text = draw(120, 40, &state).text();

    assert!(text.contains("History - Local Pangram CLI history"));
    assert!(text.contains("Showing 6"));
    assert!(text.contains("ephemeral 2026-08-09T12:00 ephemeral record"));
    assert!(text.contains("manual 2026-08-09T12:00 manual record"));
    assert!(text.contains("history 2026-08-09T12:00 history record"));
    assert!(text.contains("sixth record"));
    assert!(text.contains("Rerun is billable."));
    assert!(text.contains("[Rerun] [Export] [Delete]"));
}

#[test]
fn narrow_history_keeps_six_one_line_summaries_and_context_actions_visible() {
    let mut state = history_state(80, 24);
    state.focus = Focus::HistoryExport;
    state.history.reload(
        (0..6)
            .map(|index| {
                history_summary(
                    index,
                    SaveState::SavedHistory,
                    &format!("narrow record {index}"),
                )
            })
            .collect(),
    );

    let screen = draw(80, 24, &state);
    let text = screen.text();

    assert!(
        screen
            .row(2)
            .starts_with(" History - Local Pangram CLI history")
    );
    assert!(text.contains("narrow record 0"));
    assert!(text.contains("narrow record 5"));
    assert!(text.contains("Local history - Showing 6"));
    assert!(text.contains("Rerun is billable."));
    assert!(text.contains("[Rerun] >Export< [Delete]"));
}

#[test]
fn history_window_keeps_selection_after_the_first_six_records_visible() {
    let mut state = history_state(120, 40);
    state.focus = Focus::HistoryList;
    state.history.reload(
        (0..8)
            .map(|index| {
                history_summary(index, SaveState::SavedHistory, &format!("record-{index}"))
            })
            .collect(),
    );
    for _ in 0..6 {
        state.history.move_selection(SelectionMove::Next);
    }

    let text = draw(120, 40, &state).text();

    assert!(!text.contains("record-0"));
    assert!(text.contains("record-1"));
    assert!(text.contains("> ...1c0f8a06 succeeded AI history"));
    assert!(!text.contains("record-7"));
}

#[test]
fn history_detail_is_redacted_and_uses_the_shared_terminal_safe_result_lines() {
    let mut state = history_state(120, 24);
    state.focus = Focus::HistoryList;
    state.history.reload(vec![history_summary(
        1,
        SaveState::SavedHistory,
        "hostile detail",
    )]);
    assert!(
        state
            .history
            .load_detail(RedactedAnalysis::new(hostile_history_analysis()))
    );
    state.result_viewport.reset(history_id(1));
    state.focus = Focus::Result;

    let first_page = draw(120, 24, &state).text();

    assert!(first_page.contains("Selected detail - retained input redacted"));
    assert!(first_page.contains("Input file: report [31m.txt - text/plain [0m - 20 bytes"));
    assert!(first_page.contains("Retained input content: redacted"));
    assert!(first_page.contains("Classification: Mixed"));
    assert!(first_page.contains("Result: headline [31mforged next"));
    assert!(first_page.contains("Prediction: prediction [2Jcleared"));

    for _ in 0..3 {
        state = reduce(state, AppEvent::Key(KeyInput::PageDown)).state;
    }
    let evidence_page = draw(120, 24, &state).text();
    assert!(evidence_page.contains("8. segment-8 - 40.0% AI assistance"));
    assert!(evidence_page.contains("Plagiarism: detected - 100.0% across 8/8 sentences"));
    assert!(evidence_page.contains("https://source.test/ [31murl - matched [2Jtext"));

    state = reduce(state, AppEvent::Key(KeyInput::End)).state;
    let last_page = draw(120, 24, &state).text();
    assert!(last_page.contains("Match 8: 90.0% - https://source.test/8 - match evidence 8"));
    assert!(last_page.contains("Save state: saved history"));
    for secret in [
        "SECRET_RETAINED_FILE",
        "SECRET_EXTRACTED_TEXT",
        "SECRET_SEGMENT_TEXT",
        "/secret/original/path.txt",
    ] {
        assert!(!first_page.contains(secret), "leaked {secret}");
        assert!(!evidence_page.contains(secret), "leaked {secret}");
        assert!(!last_page.contains(secret), "leaked {secret}");
    }
    assert!(!first_page.contains('\u{1b}'));
    assert!(!evidence_page.contains('\u{1b}'));
    assert!(!last_page.contains('\u{1b}'));
}

#[test]
fn analyze_result_viewport_reaches_the_last_ordered_evidence() {
    let mut state = reduce(
        ready_state(120, 24),
        AppEvent::AnalysisFinished(hostile_history_analysis()),
    )
    .state;
    assert_eq!(state.focus, Focus::Result);

    state = reduce(state, AppEvent::Key(KeyInput::End)).state;
    let text = draw(120, 24, &state).text();

    assert!(text.contains("Match 8: 90.0% - https://source.test/8 - match evidence 8"));
    assert!(text.contains("Save state: saved history"));
}

#[test]
fn hostile_history_summary_names_are_sanitized_and_clipped_to_one_row() {
    let mut state = history_state(80, 24);
    state.focus = Focus::HistoryList;
    state.history.reload(vec![history_summary(
        1,
        SaveState::SavedHistory,
        "safe\u{1b}[31mowned\nforged-row-with-a-name-that-is-deliberately-too-long",
    )]);

    let screen = draw(80, 24, &state);
    let text = screen.text();

    assert!(text.contains("safe [31mowned forged-r"));
    assert!(!text.contains('\u{1b}'));
    assert!(!text.contains("\nforged-row"));
    assert_eq!(
        (1..16)
            .filter(|row| screen.row(*row).contains("forged-r"))
            .count(),
        1
    );
}

#[test]
fn destructive_and_export_overlays_start_on_cancel_and_name_safe_escape_paths() {
    let mut state = history_state(120, 40);
    state.history.reload(vec![history_summary(
        1,
        SaveState::SavedHistory,
        "selected record",
    )]);

    state.overlay = Some(Overlay::ConfirmHistoryDelete {
        analysis_id: history_id(1),
        confirm: false,
    });
    let delete = draw(120, 40, &state).text();
    assert!(delete.contains("Delete local history record"));
    assert!(delete.contains("> [Enter] Cancel <   [Right] Delete"));
    assert!(delete.contains("[Esc] Cancel"));

    state.overlay = Some(Overlay::HistoryExport {
        field: HistoryExportField::Action,
    });
    let export = draw(120, 40, &state).text();
    assert!(export.contains("Export local history"));
    assert!(export.contains("Format: JSONL"));
    assert!(export.contains("Content: redacted"));
    assert!(export.contains("> Action: cancel <"));
    assert!(export.contains("[Enter] Choose   [Esc] Cancel"));

    state.overlay = Some(Overlay::ConfirmFullHistoryExport {
        request: ExportRequest {
            format: HistoryExportFormat::Markdown,
            redact_content: false,
        },
        confirm: false,
    });
    let full = draw(120, 40, &state).text();
    assert!(full.contains("Export full retained content"));
    assert!(full.contains("Format: Markdown"));
    assert!(full.contains("> [Enter] Cancel <   [Right] Export full content"));
    assert!(full.contains("[Esc] Cancel"));
}
