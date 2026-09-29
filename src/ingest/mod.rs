//! Local source ingest: media transcription and GitHub prose.
//!
//! This module produces UTF-8 text. Adapters then call the shared analysis
//! module. It never talks to Pangram hosts.

#![allow(clippy::result_large_err)]

mod github;
mod media;
mod tools;

use std::path::{Path, PathBuf};

use url::Url;

use crate::domain::TextOrigin;
use crate::output::{CanonicalError, ErrorCode, Recovery};

pub(crate) use github::{GithubKind, GithubRef};
pub(crate) use media::WhisperModel;

const GITHUB_API: &str = "https://api.github.com";

#[derive(Clone, Debug)]
pub(crate) struct IngestSettings {
    pub(crate) github_api: Url,
}

impl Default for IngestSettings {
    fn default() -> Self {
        Self {
            github_api: Url::parse(GITHUB_API).expect("static GitHub API URL"),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct IngestedText {
    pub(crate) text: String,
    pub(crate) origin: TextOrigin,
    pub(crate) name: String,
    pub(crate) word_count: u64,
}

pub(crate) struct MediaOptions {
    pub(crate) data_dir: PathBuf,
    pub(crate) model: WhisperModel,
    pub(crate) download_model: bool,
    pub(crate) interactive: bool,
}
pub(crate) fn transcribe_path(
    path: &Path,
    options: MediaOptions,
) -> Result<IngestedText, CanonicalError> {
    media::transcribe_local(path, options)
}

pub(crate) fn transcribe_youtube(
    url: &str,
    options: MediaOptions,
) -> Result<IngestedText, CanonicalError> {
    media::transcribe_youtube(url, options)
}

pub(crate) fn fetch_github(
    target: &GithubRef,
    comments: bool,
    settings: &IngestSettings,
) -> Result<IngestedText, CanonicalError> {
    github::fetch(target, comments, settings)
}

pub(crate) fn usage(code: ErrorCode, message: &str) -> CanonicalError {
    CanonicalError::new(code, message).expect("ingest messages are non-empty")
}

pub(crate) fn usage_with_recovery(
    code: ErrorCode,
    message: &str,
    recovery: &str,
) -> CanonicalError {
    let recovery = Recovery::new(recovery).expect("ingest recovery is non-empty");
    usage(code, message)
        .with_recovery(recovery)
        .expect("ingest recovery is valid")
}

pub(crate) fn transcription_root(data_dir: &Path) -> PathBuf {
    data_dir.join("transcription")
}
