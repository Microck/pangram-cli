//! Compiled-binary contracts for source-ingest shortcuts. No Pangram network,
//! no live GitHub, no model download.

#[path = "support/cli_contract_env.rs"]
#[allow(dead_code)]
mod harness;

use harness::pangram;
use serde_json::Value;
use std::process::Command;

fn pangram_without_tools() -> Command {
    let mut command = pangram();
    command.env("PATH", "");
    command.env_remove("GH_TOKEN");
    command.env_remove("GITHUB_TOKEN");
    command
}

fn envelope(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout was not JSON: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[test]
fn help_lists_ingest_commands() {
    let output = pangram().arg("--help").output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    for name in ["video", "audio", "youtube", "pr", "issue", "comments"] {
        assert!(help.contains(name), "help missing {name}: {help}");
    }
}

#[test]
fn youtube_without_yt_dlp_is_missing_dependency() {
    let output = pangram_without_tools()
        .args([
            "youtube",
            "https://www.youtube.com/watch?v=aaaaaaaaaaa",
            "--max-billable-units",
            "1",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let body = envelope(&output);
    assert_eq!(body["command"], "detect");
    assert_eq!(body["error"]["code"], "missing_dependency");
    let message = body["error"]["message"]
        .as_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(message.contains("yt-dlp"), "{message}");
}

#[test]
fn video_without_ffmpeg_is_missing_dependency() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clip.mp4");
    std::fs::write(&path, b"not-a-video").unwrap();
    let output = pangram_without_tools()
        .args(["video", path.to_str().unwrap(), "--max-billable-units", "1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let body = envelope(&output);
    assert_eq!(body["command"], "detect");
    assert_eq!(body["error"]["code"], "missing_dependency");
}
#[test]
fn video_without_max_billable_units_is_unsupported_combination() {
    let output = pangram_without_tools()
        .args(["video", "missing.mp4"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let body = envelope(&output);
    assert_eq!(body["error"]["code"], "unsupported_combination");
}

#[test]
fn pr_without_github_token_is_github_authentication() {
    let output = pangram_without_tools()
        .args(["pr", "octocat/hello-world#1", "--max-billable-units", "1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let body = envelope(&output);
    assert_eq!(body["command"], "detect");
    assert_eq!(body["error"]["code"], "github_authentication");
}

#[test]
fn comments_without_github_token_is_github_authentication() {
    let output = pangram_without_tools()
        .args([
            "comments",
            "https://github.com/octocat/hello-world/pull/1",
            "--max-billable-units",
            "1",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let body = envelope(&output);
    assert_eq!(body["error"]["code"], "github_authentication");
}

#[test]
fn video_is_not_literal_detect_text() {
    let output = pangram_without_tools()
        .args(["video", "--max-billable-units", "1", "missing.mp4"])
        .output()
        .unwrap();
    let body = envelope(&output);
    assert_ne!(body["error"]["code"], "missing_api_key");
}

// Bare URL routing (contracts.md 14.12). Each routed kind is observed at the
// first local failure of its ingest path, which bare text detection can never
// produce: no credits, no GitHub, no YouTube.

fn error_code(output: &std::process::Output) -> String {
    envelope(output)["error"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("no error code: {}", String::from_utf8_lossy(&output.stdout)))
        .to_owned()
}

#[test]
fn bare_github_urls_route_to_github_ingest_with_target_flags() {
    for url in [
        "https://github.com/octocat/hello-world/pull/1",
        "https://github.com/octocat/hello-world/pull/1/files?diff=split#top",
        "https://github.com/octocat/hello-world/pull/1/commits",
        "https://github.com/octocat/hello-world/issues/2",
        "http://GitHub.com/octocat/hello-world/issues/2#issuecomment-1",
    ] {
        let output = pangram_without_tools()
            .args([url, "--comments", "--max-billable-units", "1"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(4), "{url}");
        assert!(output.stderr.is_empty(), "{url}");
        assert_eq!(envelope(&output)["command"], "detect", "{url}");
        assert_eq!(error_code(&output), "github_authentication", "{url}");
    }
}

#[test]
fn bare_github_url_keeps_the_required_ingest_ceiling() {
    let output = pangram_without_tools()
        .arg("https://github.com/octocat/hello-world/pull/1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(error_code(&output), "unsupported_combination");
}

#[test]
fn bare_github_url_after_global_flags_and_separator_still_routes() {
    let data = tempfile::tempdir().unwrap();
    let output = pangram_without_tools()
        .args([
            "--data-dir",
            data.path().to_str().unwrap(),
            "--no-color",
            "https://github.com/octocat/hello-world/pull/1",
            "--max-billable-units",
            "1",
        ])
        .output()
        .unwrap();
    assert_eq!(error_code(&output), "github_authentication");

    let output = pangram_without_tools()
        .args(["--", "https://github.com/octocat/hello-world/issues/2"])
        .output()
        .unwrap();
    assert_eq!(error_code(&output), "unsupported_combination");
}

#[test]
fn bare_youtube_urls_route_to_youtube_ingest_with_target_flags() {
    for url in [
        "https://www.youtube.com/watch?v=aaaaaaaaaaa",
        "https://youtube.com/watch?feature=share&v=aaaaaaaaaaa",
        "https://m.youtube.com/watch?v=aaaaaaaaaaa",
        "https://music.youtube.com/watch?v=aaaaaaaaaaa&list=x",
        "https://www.youtube.com/shorts/aaaaaaaaaaa",
        "https://youtu.be/aaaaaaaaaaa?t=10",
        "http://youtu.be/aaaaaaaaaaa",
    ] {
        let output = pangram_without_tools()
            .args([url, "--model", "large-v3", "--max-billable-units", "1"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{url}");
        assert_eq!(error_code(&output), "missing_dependency", "{url}");
        let message = envelope(&output)["error"]["message"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase();
        assert!(message.contains("yt-dlp"), "{url}: {message}");
    }
}

#[test]
fn bare_unsupported_url_is_unsupported_input_before_any_io() {
    for url in [
        "https://example.com/x",
        "HTTPS://example.com",
        "https://github.com/octocat/hello-world",
        "https://github.com/octocat/hello-world/pull/abc",
        "https://www.github.com/octocat/hello-world/pull/1",
        "https://www.youtube.com/@channel",
        "https://www.youtube.com/watch?list=x",
        "https://youtu.be/",
        "https://",
    ] {
        let output = pangram_without_tools().arg(url).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{url}");
        assert!(output.stderr.is_empty(), "{url}");
        let body = envelope(&output);
        assert_eq!(body["command"], "detect", "{url}");
        assert_eq!(body["error"]["code"], "unsupported_input", "{url}");
        let message = body["error"]["message"].as_str().unwrap();
        for kind in [
            "github.com/OWNER/REPO/pull/N",
            "github.com/OWNER/REPO/issues/N",
            "youtube.com/watch?v=ID",
            "youtu.be/ID",
        ] {
            assert!(message.contains(kind), "{url}: {message}");
        }
        let recovery = body["error"]["recovery"]["message"].as_str().unwrap();
        assert!(recovery.contains("pangram detect"), "{url}: {recovery}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(!stdout.contains("example.com"), "{url}: {stdout}");
        assert!(!stdout.contains("octocat"), "{url}: {stdout}");
    }
}

#[test]
fn bare_unsupported_url_honors_global_flags_and_rejects_target_flags() {
    let output = pangram_without_tools()
        .args(["https://example.com/x", "--error-format", "text"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("pangram detect"), "{stderr}");

    let output = pangram_without_tools()
        .args(["https://example.com/x", "--max-billable-units", "1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

#[test]
fn urls_outside_the_bare_url_token_stay_literal_text() {
    for arguments in [
        &["read https://example.com/x today"][..],
        &["detect", "https://example.com/x"][..],
        &["detect", "https://github.com/octocat/hello-world/pull/1"][..],
    ] {
        let output = pangram_without_tools().args(arguments).output().unwrap();
        assert_eq!(error_code(&output), "missing_api_key", "{arguments:?}");
    }
}
