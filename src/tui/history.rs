//! Pure state for the local-history TUI route.
//!
//! The runtime owns SQLite and analysis I/O. This module owns the criteria,
//! selection, privacy-bounded detail, and operation gate that screen code
//! needs to render and dispatch those effects without duplicating behavior.

use crate::domain::{
    Analysis, AnalysisId, AnalysisInput, AnalysisStatus, AnalysisSummary, CheckKind,
};
use crate::history::HistoryExportFormat;
use crate::output::CanonicalError;

use super::model::KeyInput;
use super::text_field::TextField;

pub(crate) const HISTORY_LIMIT: u32 = 50;
pub(crate) const VISIBLE_ROWS: usize = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum HistoryExportField {
    Format,
    Content,
    #[default]
    Action,
}

/// One owned request for the certified list/search worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HistoryLoadRequest {
    pub query: Option<String>,
    pub status: Option<AnalysisStatus>,
    pub check: Option<CheckKind>,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionMove {
    Previous,
    Next,
    First,
    Last,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ExportContent {
    #[default]
    Redacted,
    Full,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ExportAction {
    #[default]
    Cancel,
    Export,
}

/// The executable portion of an export choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExportRequest {
    pub format: HistoryExportFormat,
    pub redact_content: bool,
}

/// Resolving choices cannot execute a full-content export directly. The
/// reducer must handle `ConfirmFull` as the separate confirmation required by
/// the contract before starting the operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExportResolution {
    Cancel,
    Ready(ExportRequest),
    ConfirmFull(ExportRequest),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExportChoices {
    format: HistoryExportFormat,
    content: ExportContent,
    action: ExportAction,
}

impl Default for ExportChoices {
    fn default() -> Self {
        Self {
            format: HistoryExportFormat::Jsonl,
            content: ExportContent::Redacted,
            action: ExportAction::Cancel,
        }
    }
}

impl ExportChoices {
    pub(crate) const fn format(self) -> HistoryExportFormat {
        self.format
    }

    pub(crate) const fn content(self) -> ExportContent {
        self.content
    }

    pub(crate) const fn action(self) -> ExportAction {
        self.action
    }

    pub(crate) fn cycle_format(&mut self) {
        self.format = match self.format {
            HistoryExportFormat::Jsonl => HistoryExportFormat::Markdown,
            HistoryExportFormat::Markdown => HistoryExportFormat::Jsonl,
        };
    }

    pub(crate) fn toggle_content(&mut self) {
        self.content = match self.content {
            ExportContent::Redacted => ExportContent::Full,
            ExportContent::Full => ExportContent::Redacted,
        };
    }

    pub(crate) fn toggle_action(&mut self) {
        self.action = match self.action {
            ExportAction::Cancel => ExportAction::Export,
            ExportAction::Export => ExportAction::Cancel,
        };
    }

    pub(crate) const fn resolve(self) -> ExportResolution {
        if matches!(self.action, ExportAction::Cancel) {
            return ExportResolution::Cancel;
        }
        let request = ExportRequest {
            format: self.format,
            redact_content: matches!(self.content, ExportContent::Redacted),
        };
        match self.content {
            ExportContent::Redacted => ExportResolution::Ready(request),
            ExportContent::Full => ExportResolution::ConfirmFull(request),
        }
    }
}

/// Every history effect shares this one gate. Carrying the target in the
/// operation also lets stale worker responses avoid clearing a newer effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PendingOperation {
    Reload(HistoryLoadRequest),
    Detail(AnalysisId),
    Delete(AnalysisId),
    Rerun {
        original_id: AnalysisId,
        analysis_id: Option<AnalysisId>,
    },
    Export(ExportRequest),
}

impl PendingOperation {
    pub(crate) const fn rerun(original_id: AnalysisId) -> Self {
        Self::Rerun {
            original_id,
            analysis_id: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct RedactedAnalysis(Analysis<CanonicalError>);

impl RedactedAnalysis {
    pub(crate) fn new(mut analysis: Analysis<CanonicalError>) -> Self {
        // Enforce the privacy contract again at the screen-state seam. This
        // keeps an accidental `canonical_analysis(id, true)` call from
        // placing retained plaintext or original paths in TUI state.
        if let Some(input) = analysis.input.as_mut() {
            match input {
                AnalysisInput::Text(input) => input.text = None,
                AnalysisInput::File(input) => {
                    input.path = None;
                    input.extracted_text = None;
                }
            }
        }
        Self(analysis)
    }

    pub(crate) const fn analysis(&self) -> &Analysis<CanonicalError> {
        &self.0
    }
}

#[derive(Clone, Default)]
pub(crate) struct HistoryState {
    draft_query: TextField,
    applied_query: String,
    status_filter: Option<AnalysisStatus>,
    check_filter: Option<CheckKind>,
    items: Vec<AnalysisSummary>,
    selected_id: Option<AnalysisId>,
    detail: Option<RedactedAnalysis>,
    pending: Option<PendingOperation>,
    reload_dirty: bool,
    export_choices: ExportChoices,
}

impl HistoryState {
    pub(crate) fn initial_loading() -> Self {
        let mut state = Self::default();
        let request = state.load_request();
        state.start_pending(PendingOperation::Reload(request));
        state
    }

    pub(crate) fn draft_query(&self) -> &str {
        self.draft_query.value()
    }

    pub(crate) fn edit_draft_query(&mut self, key: KeyInput) -> bool {
        self.draft_query.edit(key)
    }

    /// Applies the literal draft. The runtime decides whether to start the
    /// resulting reload, so pressing Enter can refresh unchanged criteria.
    pub(crate) fn apply_query(&mut self) {
        self.applied_query.clear();
        self.applied_query.push_str(self.draft_query.value());
    }

    pub(crate) fn cycle_status_filter(&mut self) {
        self.status_filter = match self.status_filter {
            None => Some(AnalysisStatus::Queued),
            Some(AnalysisStatus::Queued) => Some(AnalysisStatus::Running),
            Some(AnalysisStatus::Running) => Some(AnalysisStatus::Succeeded),
            Some(AnalysisStatus::Succeeded) => Some(AnalysisStatus::Failed),
            Some(AnalysisStatus::Failed) => Some(AnalysisStatus::Partial),
            Some(AnalysisStatus::Partial) => None,
        };
    }

    pub(crate) fn cycle_check_filter(&mut self) {
        self.check_filter = match self.check_filter {
            None => Some(CheckKind::AiDetection),
            Some(CheckKind::AiDetection) => Some(CheckKind::Plagiarism),
            Some(CheckKind::Plagiarism) => None,
        };
    }

    pub(crate) fn load_request(&self) -> HistoryLoadRequest {
        HistoryLoadRequest {
            query: (!self.applied_query.is_empty()).then(|| self.applied_query.clone()),
            status: self.status_filter,
            check: self.check_filter,
            limit: HISTORY_LIMIT,
        }
    }

    /// Replaces the newest page while keeping ID-based selection stable.
    pub(crate) fn reload(&mut self, mut items: Vec<AnalysisSummary>) {
        items.truncate(HISTORY_LIMIT as usize);
        let selected_id = self
            .selected_id
            .filter(|selected| items.iter().any(|item| item.id == *selected))
            .or_else(|| items.first().map(|item| item.id));
        self.items = items;
        self.selected_id = selected_id;
        // A matching ID does not prove the cached detail still matches the
        // freshly certified summary. Another process may have reconciled the
        // record, so require a new detail read after every page reload.
        self.detail = None;
    }

    pub(crate) fn showing_count(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn selected_id(&self) -> Option<AnalysisId> {
        self.selected_id
    }

    pub(crate) fn selected_summary(&self) -> Option<&AnalysisSummary> {
        let selected = self.selected_id?;
        self.items.iter().find(|item| item.id == selected)
    }

    pub(crate) fn move_selection(&mut self, movement: SelectionMove) {
        if self.items.is_empty() {
            self.selected_id = None;
            self.detail = None;
            return;
        }
        let current = self
            .selected_id
            .and_then(|selected| self.items.iter().position(|item| item.id == selected))
            .unwrap_or(0);
        let next = match movement {
            SelectionMove::Previous => current.saturating_sub(1),
            SelectionMove::Next => (current + 1).min(self.items.len() - 1),
            SelectionMove::First => 0,
            SelectionMove::Last => self.items.len() - 1,
        };
        let next_id = self.items[next].id;
        if self.selected_id != Some(next_id) {
            self.selected_id = Some(next_id);
            self.detail = None;
        }
    }

    /// The selected row is always inside this derived six-row window. No
    /// mutable scroll index can drift from ID-based selection.
    pub(crate) fn scroll_offset(&self) -> usize {
        let selected = self
            .selected_id
            .and_then(|id| self.items.iter().position(|item| item.id == id))
            .unwrap_or(0);
        selected
            .saturating_add(1)
            .saturating_sub(VISIBLE_ROWS)
            .min(self.items.len().saturating_sub(VISIBLE_ROWS))
    }

    pub(crate) fn visible_items(&self) -> &[AnalysisSummary] {
        let start = self.scroll_offset();
        let end = (start + VISIBLE_ROWS).min(self.items.len());
        &self.items[start..end]
    }

    /// Accepts detail only for the currently selected ID. This rejects a
    /// stale worker response after the user moves selection.
    pub(crate) fn load_detail(&mut self, detail: RedactedAnalysis) -> bool {
        if self.selected_id != Some(detail.analysis().id) {
            return false;
        }
        self.detail = Some(detail);
        true
    }

    pub(crate) fn selected_detail(&self) -> Option<&Analysis<CanonicalError>> {
        self.detail.as_ref().map(|detail| &detail.0)
    }

    pub(crate) fn start_pending(&mut self, operation: PendingOperation) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.pending = Some(operation);
        true
    }

    /// Only the exact response that owns the gate may release it.
    pub(crate) fn finish_pending(&mut self, operation: &PendingOperation) -> bool {
        if self.pending.as_ref() != Some(operation) {
            return false;
        }
        self.pending = None;
        true
    }

    /// Binds a prepared rerun's fresh identity to the original history
    /// request that owns the operation gate. A stale preparation completion
    /// cannot replace an identity that is already running.
    pub(crate) fn bind_rerun_analysis(
        &mut self,
        original_id: AnalysisId,
        analysis_id: AnalysisId,
    ) -> bool {
        let Some(PendingOperation::Rerun {
            original_id: pending_id,
            analysis_id: pending_analysis,
        }) = self.pending.as_mut()
        else {
            return false;
        };
        if *pending_id != original_id || pending_analysis.is_some() {
            return false;
        }
        *pending_analysis = Some(analysis_id);
        true
    }

    /// Analysis worker events belong to a pending rerun only after preflight
    /// has bound its fresh identity. Other analysis completions are stale and
    /// must not alter either the global analysis state or the history gate.
    pub(crate) fn accepts_analysis_event(&self, analysis_id: AnalysisId) -> bool {
        match self.pending.as_ref() {
            Some(PendingOperation::Rerun {
                analysis_id: Some(pending_id),
                ..
            }) => *pending_id == analysis_id,
            Some(PendingOperation::Rerun { .. }) => false,
            _ => true,
        }
    }

    pub(crate) fn fail_rerun_preparation(&mut self, original_id: AnalysisId) -> bool {
        let preparing = matches!(
            self.pending.as_ref(),
            Some(PendingOperation::Rerun {
                original_id: pending_id,
                analysis_id: None,
            }) if *pending_id == original_id
        );
        if preparing {
            self.pending = None;
        }
        preparing
    }

    pub(crate) fn finish_rerun_analysis(&mut self, analysis_id: AnalysisId) -> bool {
        let running = matches!(
            self.pending.as_ref(),
            Some(PendingOperation::Rerun {
                analysis_id: Some(pending_id),
                ..
            }) if *pending_id == analysis_id
        );
        if running {
            self.pending = None;
        }
        running
    }

    pub(crate) fn pending(&self) -> Option<&PendingOperation> {
        self.pending.as_ref()
    }

    /// Records that the current page may be stale while preserving the
    /// single-operation gate. The matching completion consumes this bit and
    /// starts exactly one reload with the newest criteria.
    pub(crate) fn mark_reload_dirty(&mut self) {
        self.reload_dirty = true;
    }

    pub(crate) fn take_reload_dirty(&mut self) -> bool {
        std::mem::take(&mut self.reload_dirty)
    }

    pub(crate) const fn export_choices(&self) -> ExportChoices {
        self.export_choices
    }

    pub(crate) fn export_choices_mut(&mut self) -> &mut ExportChoices {
        &mut self.export_choices
    }

    pub(crate) fn reset_export_choices(&mut self) {
        self.export_choices = ExportChoices::default();
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use crate::domain::{
        AnalysisInputKind, Check, CheckState, FileInput, NonEmptyString, OrderedChecks, Provenance,
        Provider, SaveState, Sha256Hash, SubmissionOutcome, TextInput, TextOrigin, UtcTimestamp,
    };

    use super::*;

    fn id(index: u8) -> AnalysisId {
        format!("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a{index:02x}")
            .parse()
            .expect("canonical fixture ID")
    }

    fn summary(index: u8) -> AnalysisSummary {
        AnalysisSummary {
            id: id(index),
            status: AnalysisStatus::Succeeded,
            checks: OrderedChecks::new([CheckKind::AiDetection]).expect("one check"),
            save_state: SaveState::SavedHistory,
            input_kind: AnalysisInputKind::Text,
            display_name: Some(format!("record-{index}")),
            created_at: UtcTimestamp::from_str("2026-08-01T10:00:00Z")
                .expect("canonical timestamp"),
        }
    }

    fn summaries(indexes: impl IntoIterator<Item = u8>) -> Vec<AnalysisSummary> {
        indexes.into_iter().map(summary).collect()
    }

    #[test]
    fn reload_preserves_selected_id_or_falls_back_to_first_item() {
        let mut state = HistoryState::default();
        state.reload(summaries([1, 2, 3]));
        state.move_selection(SelectionMove::Next);
        assert_eq!(state.selected_id(), Some(id(2)));

        state.reload(summaries([4, 2, 5]));
        assert_eq!(state.selected_id(), Some(id(2)));

        state.reload(summaries([4, 5]));
        assert_eq!(state.selected_id(), Some(id(4)));

        state.reload(Vec::new());
        assert_eq!(state.selected_id(), None);
    }

    #[test]
    fn derived_window_scrolls_to_keep_selection_visible_after_six_rows() {
        let mut state = HistoryState::default();
        state.reload(summaries(0..8));
        for _ in 0..6 {
            state.move_selection(SelectionMove::Next);
        }

        assert_eq!(state.selected_id(), Some(id(6)));
        assert_eq!(state.scroll_offset(), 1);
        assert_eq!(state.visible_items().len(), VISIBLE_ROWS);
        assert_eq!(
            state.visible_items().first().map(|item| item.id),
            Some(id(1))
        );
        assert_eq!(
            state.visible_items().last().map(|item| item.id),
            Some(id(6))
        );

        state.move_selection(SelectionMove::Last);
        assert_eq!(state.scroll_offset(), 2);
        state.move_selection(SelectionMove::First);
        assert_eq!(state.scroll_offset(), 0);
    }

    #[test]
    fn filters_follow_the_exact_closed_cycles() {
        let mut state = HistoryState::default();
        let statuses = [
            Some(AnalysisStatus::Queued),
            Some(AnalysisStatus::Running),
            Some(AnalysisStatus::Succeeded),
            Some(AnalysisStatus::Failed),
            Some(AnalysisStatus::Partial),
            None,
        ];
        for expected in statuses {
            state.cycle_status_filter();
            assert_eq!(state.load_request().status, expected);
        }

        let checks = [
            Some(CheckKind::AiDetection),
            Some(CheckKind::Plagiarism),
            None,
        ];
        for expected in checks {
            state.cycle_check_filter();
            assert_eq!(state.load_request().check, expected);
        }
    }

    #[test]
    fn query_changes_only_when_the_draft_is_applied() {
        let mut state = HistoryState::default();
        for character in "first".chars() {
            assert!(state.edit_draft_query(KeyInput::Character(character)));
        }
        assert_eq!(state.load_request().query, None);

        state.apply_query();
        assert_eq!(state.load_request().query.as_deref(), Some("first"));

        for character in " second".chars() {
            assert!(state.edit_draft_query(KeyInput::Character(character)));
        }
        assert_eq!(state.load_request().query.as_deref(), Some("first"));
        state.apply_query();
        assert_eq!(state.load_request().query.as_deref(), Some("first second"));
        assert_eq!(state.load_request().limit, HISTORY_LIMIT);
    }

    #[test]
    fn one_pending_operation_rejects_every_duplicate_until_exact_finish() {
        let mut state = HistoryState::default();
        let reload = PendingOperation::Reload(state.load_request());
        assert!(state.start_pending(reload.clone()));
        assert!(!state.start_pending(reload.clone()));
        assert!(!state.start_pending(PendingOperation::Detail(id(1))));
        assert!(!state.start_pending(PendingOperation::Delete(id(1))));
        assert!(!state.start_pending(PendingOperation::rerun(id(1))));
        assert!(
            !state.start_pending(PendingOperation::Export(ExportRequest {
                format: HistoryExportFormat::Jsonl,
                redact_content: true,
            }))
        );
        assert!(!state.finish_pending(&PendingOperation::Detail(id(1))));
        assert_eq!(state.pending(), Some(&reload));
        assert!(state.finish_pending(&reload));
        assert_eq!(state.pending(), None);
    }

    #[test]
    fn rerun_gate_tracks_the_fresh_analysis_identity_until_exact_finish() {
        let original_id = id(1);
        let rerun_id = id(2);
        let unrelated_id = id(3);
        let mut state = HistoryState::default();
        assert!(state.start_pending(PendingOperation::rerun(original_id)));

        assert!(!state.accepts_analysis_event(rerun_id));
        assert!(!state.bind_rerun_analysis(unrelated_id, unrelated_id));
        assert!(state.bind_rerun_analysis(original_id, rerun_id));
        assert!(!state.fail_rerun_preparation(original_id));
        assert!(!state.accepts_analysis_event(unrelated_id));
        assert!(state.accepts_analysis_event(rerun_id));
        assert!(!state.finish_rerun_analysis(unrelated_id));
        assert!(matches!(
            state.pending(),
            Some(PendingOperation::Rerun {
                original_id: pending_id,
                analysis_id: Some(pending_analysis),
            }) if *pending_id == original_id && *pending_analysis == rerun_id
        ));
        assert!(state.finish_rerun_analysis(rerun_id));
        assert!(state.pending().is_none());
    }

    #[test]
    fn export_defaults_to_cancel_and_full_content_requires_confirmation() {
        let mut choices = ExportChoices::default();
        assert_eq!(choices.format(), HistoryExportFormat::Jsonl);
        assert_eq!(choices.content(), ExportContent::Redacted);
        assert_eq!(choices.action(), ExportAction::Cancel);
        assert_eq!(choices.resolve(), ExportResolution::Cancel);

        choices.toggle_action();
        assert_eq!(
            choices.resolve(),
            ExportResolution::Ready(ExportRequest {
                format: HistoryExportFormat::Jsonl,
                redact_content: true,
            })
        );

        choices.toggle_content();
        assert_eq!(
            choices.resolve(),
            ExportResolution::ConfirmFull(ExportRequest {
                format: HistoryExportFormat::Jsonl,
                redact_content: false,
            })
        );
        choices.cycle_format();
        assert_eq!(choices.format(), HistoryExportFormat::Markdown);
    }

    #[test]
    fn detail_state_strips_retained_text_paths_and_extracted_text() {
        let selected = id(1);
        let mut state = HistoryState::default();
        state.reload(vec![summary(1)]);

        let text = TextInput::new(
            TextOrigin::Literal,
            None,
            Sha256Hash::digest("secret"),
            6,
            1,
            Some("secret".to_owned()),
        )
        .expect("text input");
        assert!(state.load_detail(RedactedAnalysis::new(analysis(
            selected,
            AnalysisInput::Text(text),
        ))));
        let AnalysisInput::Text(text) = state
            .selected_detail()
            .and_then(|analysis| analysis.input())
            .expect("selected detail input")
        else {
            panic!("expected text input");
        };
        assert_eq!(text.text.as_deref(), None);

        let file = FileInput {
            filename: NonEmptyString::new("draft.txt").expect("filename"),
            media_type: NonEmptyString::new("text/plain").expect("media type"),
            sha256: Sha256Hash::digest("secret"),
            size_bytes: 6,
            path: Some("/private/draft.txt".to_owned()),
            extracted_text: Some("secret".to_owned()),
        };
        assert!(state.load_detail(RedactedAnalysis::new(analysis(
            selected,
            AnalysisInput::File(file),
        ))));
        let AnalysisInput::File(file) = state
            .selected_detail()
            .and_then(|analysis| analysis.input())
            .expect("selected detail input")
        else {
            panic!("expected file input");
        };
        assert_eq!(file.path.as_deref(), None);
        assert_eq!(file.extracted_text.as_deref(), None);
    }

    #[test]
    fn reload_clears_detail_even_when_the_selected_id_survives() {
        let selected = id(1);
        let mut state = HistoryState::default();
        state.reload(vec![summary(1)]);
        let text = TextInput::new(
            TextOrigin::Literal,
            None,
            Sha256Hash::digest("old"),
            3,
            1,
            Some("old".to_owned()),
        )
        .expect("text input");
        assert!(state.load_detail(RedactedAnalysis::new(analysis(
            selected,
            AnalysisInput::Text(text),
        ))));

        state.reload(vec![summary(1)]);

        assert_eq!(state.selected_id(), Some(selected));
        assert!(state.selected_detail().is_none());
    }

    fn analysis(id: AnalysisId, input: AnalysisInput) -> Analysis<CanonicalError> {
        let checks: OrderedChecks<Check<CanonicalError>> =
            OrderedChecks::new([Check::AiDetection(CheckState::Queued { upstream: None })])
                .expect("one check");
        let timestamp =
            UtcTimestamp::from_str("2026-08-01T10:00:00Z").expect("canonical timestamp");
        Analysis::new(
            id,
            SubmissionOutcome::NotSubmitted,
            input,
            checks,
            SaveState::SavedHistory,
            Provenance {
                provider: Provider::Pangram,
                upstream_version: None,
                upstream_task_ids: None,
                upstream_bulk_id: None,
                submitted_at: None,
                completed_at: None,
            },
            None,
            None,
            timestamp,
            timestamp,
            None,
        )
        .expect("valid queued analysis")
    }
}
