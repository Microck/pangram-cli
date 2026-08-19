//! The adapter-facing outcome vocabulary for one text-analysis operation.
//!
//! The adapter receives a canonical [`crate::domain::Analysis`]
//! (queued/running/terminal) or one canonical
//! [`crate::output::CanonicalError`]. No raw protocol material crosses this
//! boundary.

use crate::domain::{
    Analysis, AnalysisId, AnalysisInput, Check, FileInput, NonEmptyString, Sha256Hash, TextInput,
    TextOrigin, UpstreamTaskId,
};
use crate::output::CanonicalError;

pub const PLAGIARISM_BILLABLE_UNITS: u64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAnalysisMode {
    Detection,
    Plagiarism,
    Combined,
}

impl TextAnalysisMode {
    #[must_use]
    pub const fn billable_units(self, ai_units: u64) -> u64 {
        match self {
            Self::Detection => ai_units,
            Self::Plagiarism => PLAGIARISM_BILLABLE_UNITS,
            Self::Combined => ai_units.saturating_add(PLAGIARISM_BILLABLE_UNITS),
        }
    }

    fn from_checks(checks: &[Check<CanonicalError>]) -> Option<Self> {
        match checks {
            [Check::AiDetection(_)] => Some(Self::Detection),
            [Check::Plagiarism(_)] => Some(Self::Plagiarism),
            [Check::AiDetection(_), Check::Plagiarism(_)] => Some(Self::Combined),
            _ => None,
        }
    }
}

/// Canonical billing/input word count for submitted UTF-8 text.
#[must_use]
pub(crate) fn canonical_text_word_count(text: &str) -> u64 {
    u64::try_from(text.split_whitespace().count()).unwrap_or(u64::MAX)
}

/// Builds the exact synchronous plagiarism document used for both wire bytes
/// and ambiguity reconciliation. Keeping one constructor prevents the hash
/// from drifting from a later protocol-field change.
pub(super) fn plagiarism_body(text: &str) -> serde_json::Value {
    serde_json::json!({ "text": text })
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
    /// Returns the canonical word count when text is eligible for submission.
    /// Fresh adapter inputs and retained history inputs share this owner so no
    /// submission path can bypass the nonzero-token rule.
    pub(crate) fn eligible_text_word_count(text: &str) -> Option<u64> {
        let word_count = canonical_text_word_count(text);
        (word_count != 0).then_some(word_count)
    }

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

    /// Reconstructs a private rerun request from one saved canonical analysis.
    ///
    /// A rerun is possible only when the record retains exact text and owns a
    /// canonical AI-only, plagiarism-only, or combined check set. The retained descriptor is treated as evidence,
    /// not trusted input: its hash, byte count, word count, origin, name, and
    /// text must equal a descriptor rebuilt from the retained plaintext.
    /// Reruns always receive fresh identity and reset both plaintext output
    /// and public-link creation to their privacy-preserving defaults.
    #[must_use]
    pub fn from_saved_rerun(
        original: &Analysis<CanonicalError>,
    ) -> Option<(Self, TextAnalysisMode)> {
        let mode = TextAnalysisMode::from_checks(original.checks())?;
        let Some(AnalysisInput::Text(retained_input)) = original.input() else {
            return None;
        };
        let text = retained_input.text.as_deref()?;
        let word_count = Self::eligible_text_word_count(text)?;
        let reconstructed = TextInput::new(
            retained_input.origin(),
            retained_input.name().map(str::to_owned),
            Sha256Hash::digest(text.as_bytes()),
            u64::try_from(text.len()).unwrap_or(u64::MAX),
            word_count,
            Some(text.to_owned()),
        )
        .ok()?;
        if &reconstructed != retained_input {
            return None;
        }

        Some((
            Self::new(
                text,
                retained_input.origin(),
                retained_input.name().map(str::to_owned),
                word_count,
                false,
                false,
            )
            .with_rerun_of(original.id),
            mode,
        ))
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

    /// Hashes the exact synchronous plagiarism JSON document. The route has
    /// no model or public-link fields, so this must not reuse `submit_body`.
    #[must_use]
    pub(crate) fn plagiarism_request_sha256(&self) -> Sha256Hash {
        Sha256Hash::digest(plagiarism_body(&self.text).to_string().as_bytes())
    }
}

/// One binary file detection request with identity allocated before any
/// billable send. The path and extracted text enter canonical output only
/// when `include_input` was explicitly selected.
#[derive(Clone)]
pub struct FileAnalysisRequest {
    id: AnalysisId,
    upload: super::upstream::FileUpload,
    path: Option<String>,
    include_input: bool,
}

impl std::fmt::Debug for FileAnalysisRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileAnalysisRequest")
            .field("id", &self.id)
            .field("upload", &self.upload)
            .field("include_input", &self.include_input)
            .finish_non_exhaustive()
    }
}

impl FileAnalysisRequest {
    #[must_use]
    pub fn new(
        upload: super::upstream::FileUpload,
        path: Option<String>,
        include_input: bool,
    ) -> Self {
        Self {
            id: AnalysisId::new(),
            upload,
            path,
            include_input,
        }
    }

    #[must_use]
    pub const fn id(&self) -> AnalysisId {
        self.id
    }

    #[must_use]
    pub(crate) const fn upload(&self) -> &super::upstream::FileUpload {
        &self.upload
    }

    #[must_use]
    pub(crate) fn request_sha256(&self) -> Sha256Hash {
        self.upload.sha256()
    }

    #[must_use]
    pub(crate) const fn include_input(&self) -> bool {
        self.include_input
    }

    #[must_use]
    pub(crate) fn input(&self, extracted_text: Option<String>) -> AnalysisInput {
        AnalysisInput::File(FileInput {
            filename: NonEmptyString::new(self.upload.filename().to_owned())
                .expect("FileUpload validates a non-empty basename"),
            media_type: NonEmptyString::new(self.upload.format().media_type().to_owned())
                .expect("the closed file media type is non-empty"),
            sha256: self.upload.sha256(),
            size_bytes: self.upload.size_bytes(),
            path: self.include_input.then(|| self.path.clone()).flatten(),
            extracted_text: self.include_input.then_some(extracted_text).flatten(),
        })
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

#[cfg(test)]
mod tests {
    use crate::domain::{
        AiDetectionResult, CheckState, OrderedChecks, PlagiarismResult, Provenance, Provider,
        SaveState, SubmissionOutcome, UtcTimestamp,
    };

    use super::*;

    const RETAINED_TEXT: &str = "retained words";
    const RETAINED_NAME: &str = "saved.txt";

    fn text_input(
        text: Option<&str>,
        sha256: Sha256Hash,
        byte_count: u64,
        word_count: u64,
    ) -> TextInput {
        TextInput::new(
            TextOrigin::File,
            Some(RETAINED_NAME.to_owned()),
            sha256,
            byte_count,
            word_count,
            text.map(str::to_owned),
        )
        .unwrap()
    }

    fn valid_input() -> TextInput {
        text_input(
            Some(RETAINED_TEXT),
            Sha256Hash::digest(RETAINED_TEXT.as_bytes()),
            u64::try_from(RETAINED_TEXT.len()).unwrap(),
            2,
        )
    }

    fn ai_check() -> Check<CanonicalError> {
        let state: CheckState<AiDetectionResult, CanonicalError> =
            CheckState::Queued { upstream: None };
        Check::AiDetection(state)
    }

    fn plagiarism_check() -> Check<CanonicalError> {
        let state: CheckState<PlagiarismResult, CanonicalError> =
            CheckState::Queued { upstream: None };
        Check::Plagiarism(state)
    }

    fn saved_analysis(
        input: TextInput,
        checks: impl IntoIterator<Item = Check<CanonicalError>>,
    ) -> Analysis<CanonicalError> {
        let now = UtcTimestamp::now();
        Analysis::new(
            AnalysisId::new(),
            SubmissionOutcome::NotSubmitted,
            AnalysisInput::Text(input),
            OrderedChecks::new(checks).unwrap(),
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
            now,
            now,
            None,
        )
        .unwrap()
    }

    #[test]
    fn saved_rerun_rejects_missing_and_whitespace_only_text() {
        let missing = text_input(
            None,
            Sha256Hash::digest(RETAINED_TEXT.as_bytes()),
            u64::try_from(RETAINED_TEXT.len()).unwrap(),
            2,
        );
        assert!(
            AnalysisRequest::from_saved_rerun(&saved_analysis(missing, [ai_check()])).is_none()
        );

        let whitespace = "\u{00a0}\u{2003}\u{2029}";
        let whitespace_input = text_input(
            Some(whitespace),
            Sha256Hash::digest(whitespace.as_bytes()),
            u64::try_from(whitespace.len()).unwrap(),
            0,
        );
        assert!(
            AnalysisRequest::from_saved_rerun(&saved_analysis(whitespace_input, [ai_check()]))
                .is_none()
        );
    }

    #[test]
    fn saved_rerun_rejects_each_derived_input_integrity_mismatch() {
        let mismatches = [
            text_input(
                Some(RETAINED_TEXT),
                Sha256Hash::digest(b"different text"),
                u64::try_from(RETAINED_TEXT.len()).unwrap(),
                2,
            ),
            text_input(
                Some(RETAINED_TEXT),
                Sha256Hash::digest(RETAINED_TEXT.as_bytes()),
                999,
                2,
            ),
            text_input(
                Some(RETAINED_TEXT),
                Sha256Hash::digest(RETAINED_TEXT.as_bytes()),
                u64::try_from(RETAINED_TEXT.len()).unwrap(),
                999,
            ),
        ];

        for input in mismatches {
            assert!(
                AnalysisRequest::from_saved_rerun(&saved_analysis(input, [ai_check()])).is_none()
            );
        }
    }

    #[test]
    fn saved_rerun_preserves_each_canonical_text_check_set() {
        for (checks, expected) in [
            (vec![ai_check()], TextAnalysisMode::Detection),
            (vec![plagiarism_check()], TextAnalysisMode::Plagiarism),
            (
                vec![ai_check(), plagiarism_check()],
                TextAnalysisMode::Combined,
            ),
        ] {
            let (_, mode) =
                AnalysisRequest::from_saved_rerun(&saved_analysis(valid_input(), checks))
                    .expect("canonical text check set is rerunnable");
            assert_eq!(mode, expected);
        }
    }

    #[test]
    fn saved_rerun_creates_fresh_private_request_with_verified_identity() {
        let original = saved_analysis(valid_input(), [ai_check()]);
        let (request, mode) = AnalysisRequest::from_saved_rerun(&original).unwrap();

        assert_eq!(mode, TextAnalysisMode::Detection);
        assert_ne!(request.id(), original.id);
        assert_eq!(request.rerun_of(), Some(original.id));
        assert_eq!(request.text(), RETAINED_TEXT);
        assert_eq!(request.byte_count(), RETAINED_TEXT.len() as u64);
        assert_eq!(request.word_count(), 2);
        assert!(!request.include_input());
        assert!(!request.public_dashboard_link());
        assert_eq!(
            request.input(),
            AnalysisInput::Text(text_input(
                None,
                Sha256Hash::digest(RETAINED_TEXT.as_bytes()),
                RETAINED_TEXT.len() as u64,
                2,
            ))
        );
    }
}
