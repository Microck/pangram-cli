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
