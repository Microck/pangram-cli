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
    #[serde(default)]
    pull_request: Option<serde::de::IgnoredAny>,
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
    let repo = format!("/repos/{}/{}", target.owner, target.repo);
    // The issues endpoint serves both issues and pull requests, and marks the
    // latter with `pull_request`, so short `OWNER/REPO#N` refs resolve too.
    let item_url = api_url(settings, &format!("{repo}/issues/{}", target.number))?;
    let (item, _): (IssueOrPull, _) = get_page(&client, settings, token, item_url).await?;
    let is_pull = target.kind == GithubKind::Pull || item.pull_request.is_some();
    let mut sections = vec![
        format!("# Title\n{}", item.title.trim()),
        format!(
            "# Description\n{}",
            item.body.as_deref().unwrap_or("").trim()
        ),
    ];
    if comments {
        let path = format!("{repo}/issues/{}/comments?per_page=100", target.number);
        for comment in get_all(&client, settings, token, &path).await? {
            push_comment(&mut sections, "Comment", comment);
        }
        if is_pull {
            let path = format!("{repo}/pulls/{}/comments?per_page=100", target.number);
            for comment in get_all(&client, settings, token, &path).await? {
                push_comment(&mut sections, "Review comment", comment);
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

fn push_comment(sections: &mut Vec<String>, label: &str, comment: Comment) {
    let body = comment.body.unwrap_or_default();
    if !body.trim().is_empty() {
        sections.push(format!(
            "# {label} by {}\n{}",
            comment.user.login,
            body.trim()
        ));
    }
}

fn api_url(settings: &IngestSettings, path: &str) -> Result<Url, CanonicalError> {
    settings
        .github_api
        .join(path.trim_start_matches('/'))
        .map_err(|_| {
            usage(
                ErrorCode::UnsupportedInput,
                "could not build the GitHub request URL.",
            )
        })
}

/// Follows `Link: rel="next"` until the list is exhausted.
async fn get_all(
    client: &reqwest::Client,
    settings: &IngestSettings,
    token: &str,
    path: &str,
) -> Result<Vec<Comment>, CanonicalError> {
    let mut next = Some(api_url(settings, path)?);
    let mut all = Vec::new();
    while let Some(url) = next {
        let (page, following): (Vec<Comment>, _) = get_page(client, settings, token, url).await?;
        all.extend(page);
        next = following;
    }
    Ok(all)
}

async fn get_page<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    settings: &IngestSettings,
    token: &str,
    url: Url,
) -> Result<(T, Option<Url>), CanonicalError> {
    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| usage(ErrorCode::NetworkUnavailable, "the GitHub request failed."))?;
    let status = response.status().as_u16();
    let headers = response.headers();
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    if status == 429
        || (status == 403
            && (header("x-ratelimit-remaining") == Some("0") || header("retry-after").is_some()))
    {
        return Err(usage_with_recovery(
            ErrorCode::RateLimited,
            "GitHub rate-limited the request.",
            "Wait for the GitHub rate limit to reset, then retry.",
        ));
    }
    if status == 401 || status == 403 {
        return Err(usage_with_recovery(
            ErrorCode::GithubAuthentication,
            "GitHub rejected the configured token.",
            "Set GH_TOKEN or GITHUB_TOKEN with repo read access.",
        ));
    }
    if status == 404 {
        return Err(usage(
            ErrorCode::GithubNotFound,
            "the GitHub pull request or issue was not found.",
        ));
    }
    if !response.status().is_success() {
        return Err(usage(
            ErrorCode::NetworkUnavailable,
            "the GitHub request failed.",
        ));
    }
    let next = header("link")
        .and_then(next_link)
        .filter(|url| url.host_str() == settings.github_api.host_str());
    let body = response.json().await.map_err(|_| {
        usage(
            ErrorCode::UnsupportedInput,
            "GitHub returned an unexpected response.",
        )
    })?;
    Ok((body, next))
}

/// Extracts the `rel="next"` target from an RFC 8288 `Link` header.
fn next_link(link: &str) -> Option<Url> {
    link.split(',').find_map(|part| {
        let (target, params) = part.split_once(';')?;
        params
            .split(';')
            .any(|param| param.trim() == r#"rel="next""#)
            .then(|| Url::parse(target.trim().trim_start_matches('<').trim_end_matches('>')).ok())
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::next_link;

    #[test]
    fn next_link_selects_only_the_next_relation() {
        let header = r#"<https://api.github.com/x?page=1>; rel="prev", <https://api.github.com/x?page=3>; rel="next""#;
        assert_eq!(
            next_link(header).unwrap().as_str(),
            "https://api.github.com/x?page=3"
        );
        assert!(next_link(r#"<https://api.github.com/x?page=1>; rel="last""#).is_none());
    }
}
