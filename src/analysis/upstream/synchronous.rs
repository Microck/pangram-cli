//! Synchronous Pangram file and plagiarism protocol operations.

use std::fmt;

use tokio_util::sync::CancellationToken;

use crate::output::{CanonicalError, ErrorCode};

use super::super::config::Clock;
use super::super::http::{Response, SendOutcome};
use super::super::pacemaker::Gate as PaceGate;
use super::{
    AnalysisError, SubmitOutcome, UpstreamClient, classify_http_failure, contract_symptom_error,
    map_transport_failure,
};

/// The three file families accepted by Pangram's documented file endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFormat {
    Pdf,
    Docx,
    Rtf,
}

impl FileFormat {
    #[must_use]
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Self::Rtf => "application/rtf",
        }
    }
}

/// One validated binary document prepared for the synchronous file route.
/// Its custom debug surface never prints document bytes.
#[derive(Clone)]
pub struct FileUpload {
    filename: String,
    format: FileFormat,
    bytes: Vec<u8>,
}

impl FileUpload {
    /// Accepts a non-empty basename and non-empty bytes. Path discovery and
    /// extension selection belong to the adapter; this boundary prevents a
    /// path or empty document from reaching multipart construction.
    #[must_use]
    pub fn new(filename: impl Into<String>, format: FileFormat, bytes: Vec<u8>) -> Option<Self> {
        let filename = filename.into();
        let is_basename = !filename.is_empty()
            && !filename.contains('/')
            && !filename.contains('\\')
            && !bytes.is_empty();
        is_basename.then_some(Self {
            filename,
            format,
            bytes,
        })
    }

    #[must_use]
    pub fn filename(&self) -> &str {
        &self.filename
    }

    #[must_use]
    pub const fn format(&self) -> FileFormat {
        self.format
    }

    #[must_use]
    pub fn size_bytes(&self) -> u64 {
        u64::try_from(self.bytes.len()).unwrap_or(u64::MAX)
    }

    #[must_use]
    pub fn sha256(&self) -> crate::domain::Sha256Hash {
        crate::domain::Sha256Hash::digest(&self.bytes)
    }
}

impl fmt::Debug for FileUpload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileUpload")
            .field("filename", &self.filename)
            .field("format", &self.format)
            .field("size_bytes", &self.bytes.len())
            .finish()
    }
}

impl<C: Clock> UpstreamClient<C> {
    /// Submits one ordered batch of PDF, DOCX, or RTF files exactly once.
    /// Multipart order is request order; the response normalizer requires
    /// the same count and filename order before exposing any result.
    pub async fn submit_files(
        &self,
        files: &[FileUpload],
        cancel: &CancellationToken,
    ) -> Result<Vec<super::super::normalize::NormalizedFile>, SubmitOutcome> {
        if files.is_empty() {
            return Err(SubmitOutcome::Failed(Box::new(
                CanonicalError::new(
                    ErrorCode::UnsupportedInput,
                    "File detection requires at least one supported document.",
                )
                .expect("static template"),
            )));
        }
        match self.pacemaker.hurdle(cancel, None).await {
            PaceGate::Released => {}
            PaceGate::Cancelled | PaceGate::DeadlinePassed => {
                return Err(SubmitOutcome::Cancelled);
            }
        }
        let mut form = reqwest::multipart::Form::new().text("public_dashboard_link", "false");
        let mut expected_filenames = Vec::with_capacity(files.len());
        for file in files {
            let part = reqwest::multipart::Part::bytes(file.bytes.clone())
                .file_name(file.filename.clone())
                .mime_str(file.format.media_type())
                .expect("the closed file media types are valid MIME values");
            form = form.part("files", part);
            expected_filenames.push(file.filename.clone());
        }
        let response = classify_synchronous_send(
            self.http
                .post_multipart(&self.endpoints.file, &self.api_key, form, cancel)
                .await,
        )?;
        let body = response
            .json_value()
            .map_err(|error| SubmitOutcome::Failed(Box::new(contract_symptom_error(&error))))?;
        super::super::normalize::normalize_file_results(&body, &expected_filenames)
            .map_err(|error| SubmitOutcome::Failed(Box::new(error)))
    }

    /// Submits one text to the synchronous plagiarism route exactly once.
    /// The wire body intentionally contains only the documented `text` key.
    pub async fn submit_plagiarism(
        &self,
        text: &str,
        cancel: &CancellationToken,
    ) -> Result<crate::domain::PlagiarismResult, SubmitOutcome> {
        match self.pacemaker.hurdle(cancel, None).await {
            PaceGate::Released => {}
            PaceGate::Cancelled | PaceGate::DeadlinePassed => {
                return Err(SubmitOutcome::Cancelled);
            }
        }
        let body = super::super::task::plagiarism_body(text);
        let response = classify_synchronous_send(
            self.http
                .post_json(&self.endpoints.plagiarism, &self.api_key, &body, cancel)
                .await,
        )?;
        let body = response
            .json_value()
            .map_err(|error| SubmitOutcome::Failed(Box::new(contract_symptom_error(&error))))?;
        super::super::normalize::normalize_plagiarism(&body)
            .map_err(|error| SubmitOutcome::Failed(Box::new(error)))
    }
}

/// Applies the common no-retry semantics for synchronous billable POSTs.
/// Only a received 2xx response proceeds to normalization. A post-issue
/// cancellation or transport failure remains ambiguous.
pub(super) fn classify_synchronous_send(outcome: SendOutcome) -> Result<Response, SubmitOutcome> {
    match outcome {
        SendOutcome::Responded(response) => {
            if (200..300).contains(&response.status()) {
                Ok(response)
            } else {
                Err(SubmitOutcome::Failed(Box::new(classify_http_failure(
                    &response, None,
                ))))
            }
        }
        SendOutcome::Cancelled { issued } => {
            if issued {
                Err(SubmitOutcome::Ambiguous(AnalysisError::Cancelled))
            } else {
                Err(SubmitOutcome::Cancelled)
            }
        }
        SendOutcome::Failed {
            delivered_may_have_occurred,
            error,
        } => {
            if delivered_may_have_occurred {
                Err(SubmitOutcome::Ambiguous(error))
            } else {
                Err(SubmitOutcome::Failed(Box::new(map_transport_failure(
                    &error,
                ))))
            }
        }
    }
}
