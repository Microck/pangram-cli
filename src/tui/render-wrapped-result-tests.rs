use super::test_support::{draw, ready_state};
use super::tests::{history_id, history_state, history_summary};
use super::*;
use crate::domain::{
    Analysis, AnalysisInput, Check, CheckState, Fraction, OrderedChecks, Percentage,
    PlagiarismMatch, PlagiarismResult, SaveState, Sha256Hash, TextInput, TextOrigin,
};
use crate::output::CanonicalError;
use crate::tui::history::RedactedAnalysis;
use crate::tui::model::{AppEvent, KeyInput, reduce};

fn wrapped_result_analysis() -> Analysis<CanonicalError> {
    let match_text = format!(
        "BEGIN_SENTINEL {} TAIL_SENTINEL",
        "wide evidence ".repeat(80)
    );
    let checks = OrderedChecks::new([Check::Plagiarism(CheckState::Succeeded {
        upstream: None,
        result: PlagiarismResult {
            plagiarism_detected: true,
            total_sentences: 1,
            plagiarized_sentence_count: 1,
            percent_plagiarized: Percentage::new(100.0).expect("valid percentage"),
            matches: vec![PlagiarismMatch {
                source_url: "https://source.test/long".to_owned(),
                matched_text: match_text,
                similarity_score: Fraction::new(0.9).expect("valid fraction"),
            }],
        },
    })])
    .expect("canonical plagiarism check");
    let input = TextInput::new(
        TextOrigin::Literal,
        None,
        Sha256Hash::digest(b"wrapped result fixture"),
        22,
        3,
        None,
    )
    .expect("valid text input");
    super::tests::analysis_with_input(
        history_id(1),
        AnalysisInput::Text(input),
        checks,
        SaveState::SavedHistory,
    )
}

#[test]
fn analyze_result_viewport_reaches_every_physical_row_of_one_tall_value() {
    let mut state = reduce(
        ready_state(80, 24),
        AppEvent::AnalysisFinished(wrapped_result_analysis()),
    )
    .state;
    let mut saw_begin = false;
    let mut saw_tail = false;
    let mut saw_save = false;

    for _ in 0..80 {
        let text = draw(80, 24, &state).text();
        saw_begin |= text.contains("BEGIN_SENTINEL");
        saw_tail |= text.contains("TAIL_SENTINEL");
        saw_save |= text.contains("Save state: saved history");
        state = reduce(state, AppEvent::Key(KeyInput::Down)).state;
    }

    assert!(saw_begin, "the first physical evidence row is reachable");
    assert!(saw_tail, "the final physical evidence row is reachable");
    assert!(
        saw_save,
        "the result tail remains reachable after wrapped evidence"
    );
}

#[test]
fn history_result_viewport_reaches_wrapped_tail_and_save_state() {
    let mut state = history_state(80, 24);
    state.history.reload(vec![history_summary(
        1,
        SaveState::SavedHistory,
        "wrapped detail",
    )]);
    assert!(
        state
            .history
            .load_detail(RedactedAnalysis::new(wrapped_result_analysis()))
    );
    state.result_viewport.reset(history_id(1));
    state.focus = Focus::Result;

    let mut saw_tail = false;
    for _ in 0..80 {
        saw_tail |= draw(80, 24, &state).text().contains("TAIL_SENTINEL");
        state = reduce(state, AppEvent::Key(KeyInput::Down)).state;
    }
    assert!(saw_tail, "the final wrapped evidence row is reachable");

    state = reduce(state, AppEvent::Key(KeyInput::End)).state;
    assert!(
        draw(80, 24, &state)
            .text()
            .contains("Save state: saved history")
    );
}
