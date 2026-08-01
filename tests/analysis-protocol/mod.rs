//! Pangram 4 text protocol integration tests against a real loopback
//! Axum fixture. No mocks, no live Pangram, no credentials.
//!
//! The suite is split into three cohesive modules (submission, observation,
//! and contract matrices) over one shared support module; each file stays
//! below the repository's 800-line decomposition threshold.

mod bulk_protocol;
mod contract_matrix;
mod observation;
mod submission;

#[path = "../support/protocol_loopback/mod.rs"]
mod fixture;

mod support {
    pub(crate) use super::fixture::{
        BulkProbeClient, BulkProbeOutcome, BulkRequestView, ProtocolFixture, SYNTHETIC_KEY,
        SYNTHETIC_TEXT, Step, TASK_ID, pangram4_failure, pangram4_success,
    };
    pub(crate) use microck_pangram_cli::analysis::{
        AnalysisRequest, Analyzer, Duration, PollPolicy, RetryPolicy, StopObserving, WaitOptions,
    };
    pub(crate) use microck_pangram_cli::domain::{AnalysisStatus, TextOrigin, UpstreamTaskId};
    pub(crate) use microck_pangram_cli::output::ErrorCode;

    pub(super) const KEY_FRAGMENT: &str = "synthetic_key_0000";

    pub(super) fn request(text: &str) -> AnalysisRequest {
        AnalysisRequest::new(text, TextOrigin::Literal, None, 8, false, false)
    }

    pub(super) fn assert_scrubbed(error: &microck_pangram_cli::output::CanonicalError) {
        let rendered = format!("{error:?}");
        let serialized = serde_json::to_string(error).expect("canonical error serializes");
        for surface in [&rendered, &serialized] {
            assert!(
                !surface.contains(SYNTHETIC_KEY),
                "the synthetic key must never appear: {surface}"
            );
            assert!(
                !surface.contains(KEY_FRAGMENT),
                "even a key fragment must never appear"
            );
            assert!(
                !surface.contains(SYNTHETIC_TEXT),
                "submitted content must never appear"
            );
            assert!(
                !surface.to_ascii_lowercase().contains("x-api-key"),
                "header names stay out of errors"
            );
        }
    }
}
