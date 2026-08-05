//! The adapter-facing outcome vocabulary for one text-analysis operation.
//!
//! The adapter receives a canonical [`crate::domain::Analysis`]
//! (queued/running/terminal) or one canonical
//! [`crate::output::CanonicalError`]. No raw protocol material crosses this
//! boundary.

use crate::domain::{
    Analysis, AnalysisId, AnalysisInput, Sha256Hash, TextInput, TextOrigin, UpstreamTaskId,
};
use crate::output::CanonicalError;

/// Canonical billing/input word count for submitted UTF-8 text.
#[must_use]
pub(crate) fn canonical_text_word_count(text: &str) -> u64 {
    u64::try_from(text.split_whitespace().count()).unwrap_or(u64::MAX)
}

/// One validated text-analysis request. Construction time belongs to the
/// adapter; identity is generated here so it exists before any network call
/// and can be reported on every ambiguous or interrupted outcome.
#[derive(Debug, Clone)]
pub struct AnalysisRequest {
    id: AnalysisId,
    text: String,
    origin: TextOrigin,
    name: Option<String>,
    byte_count: u64,
    word_count: u64,
    include_input: bool,
    public_dashboard_link: bool,
    rerun_of: Option<AnalysisId>,
}

impl AnalysisRequest {
    /// A UTF-8 text submission. `word_count` is the adapter-computed
    /// canonical count used for billing estimates; `byte_count` is
    /// `text.len()`.
    #[must_use]
    pub fn new(
        text: impl Into<String>,
        origin: TextOrigin,
        name: Option<String>,
        word_count: u64,
        include_input: bool,
        public_dashboard_link: bool,
    ) -> Self {
        let text = text.into();
        let byte_count = u64::try_from(text.len()).unwrap_or(u64::MAX);
        Self {
            id: AnalysisId::new(),
            text,
            origin,
            name,
            byte_count,
            word_count,
            include_input,
            public_dashboard_link,
            rerun_of: None,
        }
    }

    /// Marks a fresh request as a rerun of one durable local analysis.
    #[must_use]
    pub fn with_rerun_of(mut self, original: AnalysisId) -> Self {
        self.rerun_of = Some(original);
        self
    }

    #[must_use]
    pub const fn rerun_of(&self) -> Option<AnalysisId> {
        self.rerun_of
    }

    #[must_use]
    pub const fn id(&self) -> AnalysisId {
        self.id
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub const fn byte_count(&self) -> u64 {
        self.byte_count
    }

    #[must_use]
    pub const fn word_count(&self) -> u64 {
        self.word_count
    }

    #[must_use]
    pub const fn include_input(&self) -> bool {
        self.include_input
    }

    #[must_use]
    pub const fn public_dashboard_link(&self) -> bool {
        self.public_dashboard_link
    }

    /// The canonical input descriptor precomputed before submission so every
    /// analysis state (including queued) carries its content hash.
    #[must_use]
    pub fn input(&self) -> AnalysisInput {
        let text_input = TextInput::new(
            self.origin,
            self.name.clone(),
            Sha256Hash::digest(self.text.as_bytes()),
            self.byte_count,
            self.word_count,
            if self.include_input {
                Some(self.text.clone())
            } else {
                None
            },
        );
        AnalysisInput::Text(text_input.expect("analysis request input construction is total"))
    }

    /// The exact JSON body sent upstream for this request. This is the single
    /// owner of the submit document: [`Self::request_sha256`] hashes precisely
    /// this value and [`UpstreamClient::submit_text`] posts precisely this
    /// value, so the reconciliation hash can never drift from the bytes on
    /// the wire (CodeRabbit data-integrity finding).
    #[must_use]
    pub fn submit_body(&self) -> serde_json::Value {
        serde_json::json!({
            "text": self.text,
            "model": "pangram-4",
            "public_dashboard_link": self.public_dashboard_link,
        })
    }

    /// The request SHA-256 used in `submission_outcome_unknown` details.
    /// Hashes the exact JSON document [`Self::submit_body`] returns.
    #[must_use]
    pub fn request_sha256(&self) -> Sha256Hash {
        Sha256Hash::digest(self.submit_body().to_string().as_bytes())
    }
}

/// A submission that reached an upstream acceptance. Carrying the full
/// input keeps the running handle able to emit every later state.
#[derive(Debug, Clone)]
pub struct AcceptedInput {
    pub task_id: UpstreamTaskId,
    pub request: AnalysisRequest,
}

/// An accepted asynchronous operation.
#[derive(Debug, Clone)]
pub enum Accepted {
    Task(AcceptedInput),
    /// Pangram returned a terminal document synchronously; the result is
    /// normalized immediately.
    Terminal(Box<Analysis<CanonicalError>>),
}

/// Every operation failure is one canonical error plus the identity needed
/// for the adapter to report or continue.
#[derive(Debug)]
pub struct TaskError {
    analysis_id: AnalysisId,
    error: CanonicalError,
}

impl TaskError {
    #[must_use]
    pub const fn new(analysis_id: AnalysisId, error: CanonicalError) -> Self {
        Self { analysis_id, error }
    }

    #[must_use]
    pub const fn canonical(&self) -> &CanonicalError {
        &self.error
    }

    #[must_use]
    pub const fn analysis_id(&self) -> AnalysisId {
        self.analysis_id
    }

    #[must_use]
    pub const fn error(&self) -> &CanonicalError {
        &self.error
    }

    #[must_use]
    pub fn into_error(self) -> CanonicalError {
        self.error
    }
}

/// The whole-operation result exposed to adapters.
pub type AnalysisResult = Result<Analysis<CanonicalError>, TaskError>;
