//! GitHub REST prose ingest. Review comments use `body` and omit `diff_hunk`.

use serde::Deserialize;
use url::Url;

use crate::analysis::canonical_text_word_count;
use crate::domain::TextOrigin;
use crate::output::{CanonicalError, ErrorCode};

use super::{IngestSettings, IngestedText, usage, usage_with_recovery};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GithubKind {
    Pull,
    Issue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GithubRef {
    pub(crate) owner: String,
    pub(crate) repo: String,
    pub(crate) number: u64,
    pub(crate) kind: GithubKind,
}

impl GithubRef {
    pub(crate) fn force_kind(mut self, kind: GithubKind) -> Self {
        self.kind = kind;
        self
    }

    pub(crate) fn parse(raw: &str) -> Result<Self, CanonicalError> {
        let raw = raw.trim();
        if let Some(parsed) = parse_url(raw)? {
            return Ok(parsed);
        }
        parse_short(raw)
    }
}

fn parse_url(raw: &str) -> Result<Option<GithubRef>, CanonicalError> {
    let Ok(url) = Url::parse(raw) else {
        return Ok(None);
    };
    if url.host_str() != Some("github.com") {
        return Err(usage(
            ErrorCode::UnsupportedInput,
            "GitHub references must use github.com.",
        ));
    }
    let mut segments = url
        .path_segments()
        .ok_or_else(|| usage(ErrorCode::UnsupportedInput, "the GitHub URL path is empty."))?
        .filter(|segment| !segment.is_empty());
    let owner = segments.next().ok_or_else(|| {
        usage(
            ErrorCode::UnsupportedInput,
            "the GitHub URL is missing an owner.",
        )
    })?;
    let repo = segments.next().ok_or_else(|| {
        usage(
            ErrorCode::UnsupportedInput,
            "the GitHub URL is missing a repository.",
        )
    })?;
    let kind = match segments.next() {
        Some("pull") => GithubKind::Pull,
        Some("issues") => GithubKind::Issue,
        _ => {
            return Err(usage(
                ErrorCode::UnsupportedInput,
                "the GitHub URL must point at a pull request or issue.",
            ));
        }
    };
    let number = segments
        .next()
        .ok_or_else(|| {
            usage(
                ErrorCode::UnsupportedInput,
                "the GitHub URL is missing a number.",
            )
        })?
        .parse::<u64>()
        .map_err(|_| usage(ErrorCode::UnsupportedInput, "the GitHub number is invalid."))?;
    Ok(Some(GithubRef {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        number,
        kind,
    }))
}

fn parse_short(raw: &str) -> Result<GithubRef, CanonicalError> {
    let (repo_part, number_part) = raw.split_once('#').ok_or_else(|| {
        usage(
            ErrorCode::UnsupportedInput,
            "use OWNER/REPO#N or a github.com pull/issue URL.",
        )
    })?;
    let (owner, repo) = repo_part.split_once('/').ok_or_else(|| {
        usage(
            ErrorCode::UnsupportedInput,
            "use OWNER/REPO#N or a github.com pull/issue URL.",
        )
    })?;
    let number = number_part
        .parse::<u64>()
        .map_err(|_| usage(ErrorCode::UnsupportedInput, "the GitHub number is invalid."))?;
    if owner.is_empty() || repo.is_empty() {
        return Err(usage(
            ErrorCode::UnsupportedInput,
            "use OWNER/REPO#N or a github.com pull/issue URL.",
        ));
    }
    Ok(GithubRef {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        number,
        kind: GithubKind::Issue,
    })
}

#[derive(Deserialize)]
struct Actor {
    login: String,
}

#[derive(Deserialize)]
struct IssueOrPull {
    html_url: String,
    title: String,
    body: Option<String>,
}

#[derive(Deserialize)]
struct Comment {
    body: Option<String>,
    user: Actor,
}

pub(super) fn fetch(
    target: &GithubRef,
    comments: bool,
    settings: &IngestSettings,
) -> Result<IngestedText, CanonicalError> {
    let token = github_token()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| {
            usage(
                ErrorCode::NetworkUnavailable,
                "could not start the local GitHub runtime.",
            )
        })?;
    runtime.block_on(fetch_async(target, comments, settings, &token))
}

fn github_token() -> Result<String, CanonicalError> {
    for key in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_owned());
            }
        }
    }
    Err(usage_with_recovery(
        ErrorCode::GithubAuthentication,
        "no GitHub token is configured.",
        "Set GH_TOKEN or GITHUB_TOKEN.",
    ))
}

async fn fetch_async(
    target: &GithubRef,
    comments: bool,
    settings: &IngestSettings,
    token: &str,
) -> Result<IngestedText, CanonicalError> {
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .user_agent(concat!("pangram-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| {
            usage(
                ErrorCode::NetworkUnavailable,
                "could not build the GitHub client.",
            )
        })?;
    let resource_path = match target.kind {
        GithubKind::Pull => format!(
            "/repos/{}/{}/pulls/{}",
            target.owner, target.repo, target.number
        ),
        GithubKind::Issue => format!(
            "/repos/{}/{}/issues/{}",
            target.owner, target.repo, target.number
        ),
    };
    let item: IssueOrPull = get_json(&client, settings, token, &resource_path).await?;
    let mut sections = vec![
        format!("# Title\n{}", item.title.trim()),
        format!(
            "# Description\n{}",
            item.body.as_deref().unwrap_or("").trim()
        ),
    ];
    if comments {
        let issue_comments: Vec<Comment> = get_json(
            &client,
            settings,
            token,
            &format!(
                "/repos/{}/{}/issues/{}/comments?per_page=100",
                target.owner, target.repo, target.number
            ),
        )
        .await?;
        for comment in issue_comments {
            let body = comment.body.unwrap_or_default();
            if body.trim().is_empty() {
                continue;
            }
            sections.push(format!(
                "# Comment by {}\n{}",
                comment.user.login,
                body.trim()
            ));
        }
        if target.kind == GithubKind::Pull {
            let review_comments: Vec<Comment> = get_json(
                &client,
                settings,
                token,
                &format!(
                    "/repos/{}/{}/pulls/{}/comments?per_page=100",
                    target.owner, target.repo, target.number
                ),
            )
            .await?;
            for comment in review_comments {
                let body = comment.body.unwrap_or_default();
                if body.trim().is_empty() {
                    continue;
                }
                sections.push(format!(
                    "# Review comment by {}\n{}",
                    comment.user.login,
                    body.trim()
                ));
            }
        }
    }
    let text = sections.join("\n\n");
    let word_count = canonical_text_word_count(&text);
    if word_count == 0 {
        return Err(usage(
            ErrorCode::UnsupportedInput,
            "the GitHub item contained no words.",
        ));
    }
    Ok(IngestedText {
        text,
        origin: TextOrigin::Github,
        name: item.html_url,
        word_count,
    })
}

async fn get_json<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    settings: &IngestSettings,
    token: &str,
    path: &str,
) -> Result<T, CanonicalError> {
    let url = settings
        .github_api
        .join(path.trim_start_matches('/'))
        .map_err(|_| {
            usage(
                ErrorCode::UnsupportedInput,
                "could not build the GitHub request URL.",
            )
        })?;
    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| usage(ErrorCode::NetworkUnavailable, "the GitHub request failed."))?;
    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(usage_with_recovery(
            ErrorCode::GithubAuthentication,
            "GitHub rejected the configured token.",
            "Set GH_TOKEN or GITHUB_TOKEN with repo read access.",
        ));
    }
    if status.as_u16() == 404 {
        return Err(usage(
            ErrorCode::GithubNotFound,
            "the GitHub pull request or issue was not found.",
        ));
    }
    if !status.is_success() {
        return Err(usage(
            ErrorCode::NetworkUnavailable,
            "the GitHub request failed.",
        ));
    }
    response.json().await.map_err(|_| {
        usage(
            ErrorCode::UnsupportedInput,
            "GitHub returned an unexpected response.",
        )
    })
}
