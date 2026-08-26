//! Derived projection for unfinished in-session and saved analyses.
//!
//! Saved rows are reconciled from a complete certified unfinished projection,
//! independent of the filtered and limited History display page.

use std::collections::HashSet;

use crate::domain::{Analysis, AnalysisId, AnalysisStatus, AnalysisSummary};
use crate::output::CanonicalError;

use super::model::{AppState, Focus, KeyInput, Keymap, Route};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ActiveSource {
    Session,
    Saved,
}

pub(super) fn reduce_key(state: &mut AppState, key: KeyInput) -> bool {
    if state.route != Route::Active || state.focus != Focus::ActiveList {
        return false;
    }
    let movement = match key {
        KeyInput::Up => SelectionMove::Previous,
        KeyInput::Down => SelectionMove::Next,
        KeyInput::Home => SelectionMove::First,
        KeyInput::End => SelectionMove::Last,
        KeyInput::Character('k') if state.keymap == Keymap::Vim => SelectionMove::Previous,
        KeyInput::Character('j') if state.keymap == Keymap::Vim => SelectionMove::Next,
        _ => return false,
    };
    state.active.move_selection(movement);
    true
}

impl ActiveSource {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Session => "this session",
            Self::Saved => "saved history",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ActiveRow {
    pub(super) id: AnalysisId,
    pub(super) status: AnalysisStatus,
    pub(super) source: ActiveSource,
}

const VISIBLE_ROWS: usize = 6;

#[derive(Clone, Default)]
pub(super) struct ActiveState {
    rows: Vec<ActiveRow>,
    selected_id: Option<AnalysisId>,
}

#[derive(Clone, Copy)]
enum SelectionMove {
    Previous,
    Next,
    First,
    Last,
}

impl ActiveState {
    pub(super) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(super) fn has_session(&self) -> bool {
        self.rows
            .iter()
            .any(|row| row.source == ActiveSource::Session)
    }

    pub(super) fn selected_id(&self) -> Option<AnalysisId> {
        self.selected_id
    }

    pub(super) fn visible_rows(&self) -> &[ActiveRow] {
        let selected = self
            .selected_id
            .and_then(|id| self.rows.iter().position(|row| row.id == id))
            .unwrap_or(0);
        let start = selected
            .saturating_add(1)
            .saturating_sub(VISIBLE_ROWS)
            .min(self.rows.len().saturating_sub(VISIBLE_ROWS));
        let end = (start + VISIBLE_ROWS).min(self.rows.len());
        &self.rows[start..end]
    }

    pub(super) fn select_visible(&mut self, index: usize) -> bool {
        let Some(row) = self.visible_rows().get(index) else {
            return false;
        };
        self.selected_id = Some(row.id);
        true
    }

    #[cfg(test)]
    pub(super) fn status(&self, analysis_id: AnalysisId) -> Option<AnalysisStatus> {
        self.rows
            .iter()
            .find(|row| row.id == analysis_id)
            .map(|row| row.status)
    }

    pub(super) fn accept(&mut self, analysis: &Analysis<CanonicalError>) {
        if let Some(index) = self.rows.iter().position(|row| row.id == analysis.id) {
            self.rows.remove(index);
        }
        self.rows.insert(
            0,
            ActiveRow {
                id: analysis.id,
                status: analysis.status(),
                source: ActiveSource::Session,
            },
        );
        self.selected_id = Some(analysis.id);
    }

    pub(super) fn progress(&mut self, analysis_id: AnalysisId) -> bool {
        let Some(row) = self.rows.iter_mut().find(|row| row.id == analysis_id) else {
            return false;
        };
        row.status = AnalysisStatus::Running;
        true
    }

    pub(super) fn merge_saved(&mut self, summaries: &[AnalysisSummary]) {
        for summary in summaries {
            let unfinished = matches!(
                summary.status,
                AnalysisStatus::Queued | AnalysisStatus::Running
            );
            let Some(index) = self.rows.iter().position(|row| row.id == summary.id) else {
                if unfinished {
                    self.rows.push(ActiveRow {
                        id: summary.id,
                        status: summary.status,
                        source: ActiveSource::Saved,
                    });
                    self.selected_id.get_or_insert(summary.id);
                }
                continue;
            };
            if !unfinished {
                self.remove_at(index);
                continue;
            }

            let row = &mut self.rows[index];
            // A delayed saved snapshot must not regress fresher in-session
            // progress from running back to queued.
            if row.source == ActiveSource::Saved || summary.status == AnalysisStatus::Running {
                row.status = summary.status;
            }
        }
    }

    /// Reconciles saved rows against the complete durable unfinished set.
    /// Session-owned rows survive because they may be intentionally ephemeral.
    pub(super) fn replace_saved(&mut self, summaries: &[AnalysisSummary]) {
        let unfinished = summaries
            .iter()
            .map(|summary| summary.id)
            .collect::<HashSet<_>>();
        self.rows
            .retain(|row| row.source == ActiveSource::Session || unfinished.contains(&row.id));
        if self
            .selected_id
            .is_some_and(|selected| !self.rows.iter().any(|row| row.id == selected))
        {
            self.selected_id = self.rows.first().map(|row| row.id);
        }
        self.merge_saved(summaries);
    }

    pub(super) fn remove(&mut self, analysis_id: AnalysisId) {
        let Some(index) = self.rows.iter().position(|row| row.id == analysis_id) else {
            return;
        };
        self.remove_at(index);
    }

    fn remove_at(&mut self, index: usize) {
        let analysis_id = self.rows[index].id;
        self.rows.remove(index);
        if self.selected_id == Some(analysis_id) {
            self.selected_id = self
                .rows
                .get(index)
                .or_else(|| self.rows.last())
                .map(|row| row.id);
        }
    }

    fn move_selection(&mut self, movement: SelectionMove) {
        if self.rows.is_empty() {
            self.selected_id = None;
            return;
        }
        let current = self
            .selected_id
            .and_then(|id| self.rows.iter().position(|row| row.id == id))
            .unwrap_or(0);
        let next = match movement {
            SelectionMove::Previous => current.saturating_sub(1),
            SelectionMove::Next => (current + 1).min(self.rows.len() - 1),
            SelectionMove::First => 0,
            SelectionMove::Last => self.rows.len() - 1,
        };
        self.selected_id = Some(self.rows[next].id);
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use crate::domain::{AnalysisInputKind, CheckKind, OrderedChecks, SaveState, UtcTimestamp};

    use super::*;

    fn analysis_id(index: u8) -> AnalysisId {
        format!("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f9a{index:02x}")
            .parse()
            .expect("canonical fixture ID")
    }

    fn summary(index: u8, status: AnalysisStatus) -> AnalysisSummary {
        AnalysisSummary {
            id: analysis_id(index),
            status,
            checks: OrderedChecks::new([CheckKind::AiDetection]).expect("one check"),
            save_state: SaveState::SavedHistory,
            input_kind: AnalysisInputKind::Text,
            display_name: None,
            created_at: UtcTimestamp::from_str("2026-08-01T10:00:00Z")
                .expect("canonical timestamp"),
        }
    }

    #[test]
    fn saved_pages_do_not_regress_session_progress_but_terminal_evidence_removes_it() {
        let id = analysis_id(1);
        let mut active = ActiveState {
            rows: vec![ActiveRow {
                id,
                status: AnalysisStatus::Running,
                source: ActiveSource::Session,
            }],
            selected_id: Some(id),
        };

        active.merge_saved(&[summary(1, AnalysisStatus::Queued)]);
        assert_eq!(active.status(id), Some(AnalysisStatus::Running));
        assert_eq!(active.len(), 1);

        active.merge_saved(&[summary(1, AnalysisStatus::Succeeded)]);
        assert_eq!(active.status(id), None);
    }

    #[test]
    fn selection_reaches_every_row_and_follows_exact_removal() {
        let mut active = ActiveState::default();
        active.merge_saved(
            &(1..=8)
                .map(|index| summary(index, AnalysisStatus::Queued))
                .collect::<Vec<_>>(),
        );

        active.move_selection(SelectionMove::Last);
        assert_eq!(active.selected_id(), Some(analysis_id(8)));
        assert_eq!(
            active.visible_rows().first().map(|row| row.id),
            Some(analysis_id(3))
        );
        assert_eq!(
            active.visible_rows().last().map(|row| row.id),
            Some(analysis_id(8))
        );

        active.remove(analysis_id(8));
        assert_eq!(active.selected_id(), Some(analysis_id(7)));
    }
}
