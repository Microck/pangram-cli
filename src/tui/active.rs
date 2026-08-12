//! Derived projection for unfinished in-session and saved analyses.
//!
//! Saved rows are merged from certified history pages. A filtered or limited
//! page may omit an existing saved row, so omission is not completion proof.

use crate::domain::{Analysis, AnalysisId, AnalysisStatus, AnalysisSummary};
use crate::output::CanonicalError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ActiveSource {
    Session,
    Saved,
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

#[derive(Clone, Default)]
pub(super) struct ActiveState(Vec<ActiveRow>);

impl ActiveState {
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    pub(super) fn has_session(&self) -> bool {
        self.0.iter().any(|row| row.source == ActiveSource::Session)
    }

    pub(super) fn rows(&self) -> &[ActiveRow] {
        &self.0
    }

    #[cfg(test)]
    pub(super) fn status(&self, analysis_id: AnalysisId) -> Option<AnalysisStatus> {
        self.0
            .iter()
            .find(|row| row.id == analysis_id)
            .map(|row| row.status)
    }

    pub(super) fn accept(&mut self, analysis: &Analysis<CanonicalError>) {
        if let Some(row) = self.0.iter_mut().find(|row| row.id == analysis.id) {
            row.status = analysis.status();
            row.source = ActiveSource::Session;
        } else {
            self.0.push(ActiveRow {
                id: analysis.id,
                status: analysis.status(),
                source: ActiveSource::Session,
            });
        }
    }

    pub(super) fn progress(&mut self, analysis_id: AnalysisId) -> bool {
        let Some(row) = self.0.iter_mut().find(|row| row.id == analysis_id) else {
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
            let Some(index) = self.0.iter().position(|row| row.id == summary.id) else {
                if unfinished {
                    self.0.push(ActiveRow {
                        id: summary.id,
                        status: summary.status,
                        source: ActiveSource::Saved,
                    });
                }
                continue;
            };
            if !unfinished {
                self.0.remove(index);
                continue;
            }

            let row = &mut self.0[index];
            // A delayed saved snapshot must not regress fresher in-session
            // progress from running back to queued.
            if row.source == ActiveSource::Saved || summary.status == AnalysisStatus::Running {
                row.status = summary.status;
            }
        }
    }

    pub(super) fn remove(&mut self, analysis_id: AnalysisId) {
        self.0.retain(|row| row.id != analysis_id);
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
        let mut active = ActiveState(vec![ActiveRow {
            id,
            status: AnalysisStatus::Running,
            source: ActiveSource::Session,
        }]);

        active.merge_saved(&[summary(1, AnalysisStatus::Queued)]);
        assert_eq!(active.status(id), Some(AnalysisStatus::Running));
        assert_eq!(active.len(), 1);

        active.merge_saved(&[summary(1, AnalysisStatus::Succeeded)]);
        assert_eq!(active.status(id), None);
    }
}
