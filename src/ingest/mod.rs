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

/// The ingest command a bare root URL selects (contracts.md 14.12).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UrlRoute {
    Pull,
    Issue,
    Youtube,
    Unsupported,
}

impl UrlRoute {
    /// The routed subcommand name, or `None` for an unsupported URL.
    pub(crate) fn command(self) -> Option<&'static str> {
        match self {
            Self::Pull => Some("pr"),
            Self::Issue => Some("issue"),
            Self::Youtube => Some("youtube"),
            Self::Unsupported => None,
        }
    }
}

/// Whether a bare argument is one URL token: an `http://` or `https://`
/// prefix (ASCII case-insensitive) and no whitespace.
pub(crate) fn is_url_token(raw: &str) -> bool {
    let has_prefix = |prefix: &str| {
        raw.get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    };
    (has_prefix("http://") || has_prefix("https://")) && !raw.chars().any(char::is_whitespace)
}

/// Classifies a bare argument; `None` means it is not a URL token. GitHub
/// items reuse the `REF` parser so routing and the target agree exactly.
pub(crate) fn route_url(raw: &str) -> Option<UrlRoute> {
    if !is_url_token(raw) {
        return None;
    }
    let Ok(url) = Url::parse(raw) else {
        return Some(UrlRoute::Unsupported);
    };
    let route = match url.host_str() {
        Some("github.com") => match GithubRef::parse(raw) {
            Ok(target) if target.kind == GithubKind::Pull => UrlRoute::Pull,
            Ok(_) => UrlRoute::Issue,
            Err(_) => UrlRoute::Unsupported,
        },
        _ if is_youtube_video(&url) => UrlRoute::Youtube,
        _ => UrlRoute::Unsupported,
    };
    Some(route)
}

fn is_youtube_video(url: &Url) -> bool {
    let mut segments = url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|segment| !segment.is_empty());
    match url.host_str() {
        Some("youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com") => {
            match segments.next() {
                Some("watch") => url
                    .query_pairs()
                    .any(|(key, value)| key == "v" && !value.is_empty()),
                Some("shorts") => segments.next().is_some(),
                _ => false,
            }
        }
        Some("youtu.be") => segments.next().is_some(),
        _ => false,
    }
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
