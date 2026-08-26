use super::tests::{assert_pending_rerun, history_id, history_state, history_summary};
use super::*;

#[test]
fn rerun_gate_stays_closed_until_the_analysis_finishes() {
    let mut state = history_state(vec![history_summary(1)]);
    state.route = Route::History;
    state.focus = Focus::HistoryRerun;
    let analysis_id = history_id(1);
    let rerun_id = AnalysisId::new();

    let requested = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(matches!(
        requested.effects.as_slice(),
        [Effect::PrepareHistoryRerun {
            analysis_id: requested_id,
            automatic_save: false,
        }] if *requested_id == analysis_id
    ));
    assert_pending_rerun(&requested.state, analysis_id, None);
    assert!(requested.state.analysis.submitting);

    let mut preparing = requested.state;
    preparing.route = Route::Analyze;
    preparing.focus = Focus::Submit;
    preparing.composer = TextField::from_value("second request".to_owned());
    let fresh_while_preparing = reduce(preparing, AppEvent::Key(KeyInput::Enter));
    assert!(fresh_while_preparing.effects.is_empty());

    let mut preparing = fresh_while_preparing.state;
    preparing.route = Route::History;
    preparing.focus = Focus::HistoryRerun;
    let rerun_while_preparing = reduce(preparing, AppEvent::Key(KeyInput::Enter));
    assert!(rerun_while_preparing.effects.is_empty());

    let wrong = reduce(
        rerun_while_preparing.state,
        AppEvent::HistoryRerunPrepared {
            analysis_id: history_id(2),
            result: Ok(AnalysisId::new()),
        },
    );
    assert_pending_rerun(&wrong.state, analysis_id, None);
    let completed = reduce(
        wrong.state,
        AppEvent::HistoryRerunPrepared {
            analysis_id,
            result: Ok(rerun_id),
        },
    );
    assert_eq!(completed.state.route, Route::Active);
    assert_eq!(completed.state.focus, Focus::ActiveList);
    assert_eq!(completed.state.notice.as_deref(), Some("Rerun started."));
    assert!(completed.state.analysis.submitting);
    assert_pending_rerun(&completed.state, analysis_id, Some(rerun_id));

    let stale_preflight_failure = reduce(
        completed.state,
        AppEvent::HistoryRerunPrepared {
            analysis_id,
            result: Err(CanonicalError::new(
                crate::output::ErrorCode::LocalTaskUnresolvable,
                "stale preflight failure",
            )
            .expect("valid canonical error")),
        },
    );
    assert!(stale_preflight_failure.state.analysis.submitting);
    assert_pending_rerun(&stale_preflight_failure.state, analysis_id, Some(rerun_id));

    let mut returned = stale_preflight_failure.state;
    returned.route = Route::History;
    returned.focus = Focus::HistoryRerun;
    let duplicate = reduce(returned, AppEvent::Key(KeyInput::Enter));
    assert!(duplicate.effects.is_empty());
    assert_pending_rerun(&duplicate.state, analysis_id, Some(rerun_id));

    let unrelated_failure = reduce(
        duplicate.state,
        AppEvent::AnalysisFailed(AnalysisFailure {
            analysis_id: AnalysisId::new(),
            error: CanonicalError::new(crate::output::ErrorCode::NetworkUnavailable, "offline")
                .expect("valid canonical error"),
        }),
    );
    assert!(unrelated_failure.state.analysis.submitting);
    assert!(unrelated_failure.state.analysis.failure.is_none());
    assert_pending_rerun(&unrelated_failure.state, analysis_id, Some(rerun_id));

    let mut attempted_fresh_submit = unrelated_failure.state;
    attempted_fresh_submit.route = Route::Analyze;
    attempted_fresh_submit.focus = Focus::Submit;
    attempted_fresh_submit.composer = TextField::from_value("second request".to_owned());
    let blocked = reduce(attempted_fresh_submit, AppEvent::Key(KeyInput::Enter));
    assert!(blocked.effects.is_empty());
    assert_eq!(
        blocked.state.notice.as_deref(),
        Some("An analysis is already in progress.")
    );

    let failed = reduce(
        blocked.state,
        AppEvent::AnalysisFailed(AnalysisFailure {
            analysis_id: rerun_id,
            error: CanonicalError::new(crate::output::ErrorCode::NetworkUnavailable, "offline")
                .expect("valid canonical error"),
        }),
    );
    assert!(!failed.state.analysis.submitting);
    assert!(failed.state.history.pending().is_none());
    assert_eq!(failed.state.route, Route::Analyze);
    assert_eq!(failed.state.focus, Focus::Submit);
    assert_eq!(failed.state.notice, None);
}
