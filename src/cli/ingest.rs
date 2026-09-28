//! CLI dispatch for source-ingest shortcuts.

#![allow(clippy::result_large_err)]

use std::path::PathBuf;

use clap::{Arg, ArgAction, Command};

use crate::ingest::{
    GithubKind, GithubRef, IngestedText, MediaOptions, WhisperModel, fetch_github, transcribe_path,
    transcribe_youtube, usage,
};
use crate::output::{CanonicalError, ErrorCode};

fn ingest_flags(command: Command) -> Command {
    command
        .arg(
            Arg::new("format")
                .long("format")
                .value_name("FORMAT")
                .value_parser(["json", "jsonl", "toon", "markdown", "pretty"])
                .help("Render the canonical envelope in the selected projection"),
        )
        .arg(
            Arg::new("include-input")
                .long("include-input")
                .action(ArgAction::SetTrue)
                .help("Include submitted content in the canonical input record"),
        )
        .arg(
            Arg::new("save")
                .long("save")
                .action(ArgAction::SetTrue)
                .help("Persist this analysis in local history, even while automatic history is disabled"),
        )
        .arg(
            Arg::new("timeout")
                .long("timeout")
                .value_name("DURATION")
                .num_args(1)
                .help("Bound the wait (seconds, or a value with an s, ms, m, or h suffix)"),
        )
        .arg(
            Arg::new("progress")
                .long("progress")
                .value_name("MODE")
                .value_parser(["auto", "never", "jsonl"])
                .help("Progress reporting on stderr: auto, never, or canonical jsonl"),
        )
        .arg(
            Arg::new("max-billable-units")
                .long("max-billable-units")
                .value_name("N")
                .num_args(1)
                .help("Reject the request when the estimated cost exceeds this ceiling"),
        )
        .arg(
            Arg::new("detach")
                .long("detach")
                .action(ArgAction::SetTrue)
                .conflicts_with("save")
                .help("Report the accepted task without waiting for the result"),
        )
        .arg(
            Arg::new("public-link")
                .long("public-link")
                .action(ArgAction::SetTrue)
                .help("Ask Pangram to create a public dashboard link for this analysis"),
        )
}

fn media_flags(command: Command) -> Command {
    ingest_flags(command)
        .arg(
            Arg::new("model")
                .long("model")
                .value_name("MODEL")
                .value_parser(["large-v3-turbo", "large-v3-turbo-q5_0", "large-v3"])
                .help("Local Whisper ggml model (default large-v3-turbo)"),
        )
        .arg(
            Arg::new("download-model")
                .long("download-model")
                .action(ArgAction::SetTrue)
                .help("Download the selected ggml weights after confirmation or in this noninteractive run"),
        )
}

fn github_flags(command: Command) -> Command {
    ingest_flags(command).arg(
        Arg::new("comments")
            .long("comments")
            .action(ArgAction::SetTrue)
            .help("Include issue comments and pull-request review bodies"),
    )
}

pub(crate) fn video_command() -> Command {
    media_flags(
        Command::new("video")
            .about("Transcribe a local video and run Pangram 4 AI detection")
            .arg(
                Arg::new("PATH")
                    .value_name("PATH")
                    .required(true)
                    .help("Local video file"),
            ),
    )
}

pub(crate) fn audio_command() -> Command {
    media_flags(
        Command::new("audio")
            .about("Transcribe a local audio file and run Pangram 4 AI detection")
            .arg(
                Arg::new("PATH")
                    .value_name("PATH")
                    .required(true)
                    .help("Local audio file"),
            ),
    )
}

pub(crate) fn youtube_command() -> Command {
    media_flags(
        Command::new("youtube")
            .about("Download audio with yt-dlp, transcribe, and run Pangram 4 AI detection")
            .arg(
                Arg::new("URL")
                    .value_name("URL")
                    .required(true)
                    .help("YouTube URL"),
            ),
    )
}

pub(crate) fn pr_command() -> Command {
    github_flags(
        Command::new("pr")
            .about("Detect AI writing in a GitHub pull request title and body")
            .arg(
                Arg::new("REF")
                    .value_name("REF")
                    .required(true)
                    .help("Pull request URL or OWNER/REPO#N"),
            ),
    )
}

pub(crate) fn issue_command() -> Command {
    github_flags(
        Command::new("issue")
            .about("Detect AI writing in a GitHub issue title and body")
            .arg(
                Arg::new("REF")
                    .value_name("REF")
                    .required(true)
                    .help("Issue URL or OWNER/REPO#N"),
            ),
    )
}

pub(crate) fn comments_command() -> Command {
    ingest_flags(
        Command::new("comments")
            .about("Detect AI writing in GitHub issue and review comment bodies")
            .arg(
                Arg::new("REF")
                    .value_name("REF")
                    .required(true)
                    .help("Pull or issue URL, or OWNER/REPO#N"),
            ),
    )
}

pub(crate) fn ingest(
    name: &str,
    matches: &clap::ArgMatches,
    root_matches: &clap::ArgMatches,
    interactive: bool,
    settings: &crate::ingest::IngestSettings,
) -> Result<IngestedText, CanonicalError> {
    if matches.get_one::<String>("max-billable-units").is_none() {
        return Err(usage(
            ErrorCode::UnsupportedCombination,
            "--max-billable-units is required for ingest commands.",
        ));
    }
    match name {
        "video" | "audio" => {
            let path = PathBuf::from(matches.get_one::<String>("PATH").expect("PATH is required"));
            transcribe_path(&path, media_options(matches, root_matches, interactive)?)
        }
        "youtube" => {
            let url = matches.get_one::<String>("URL").expect("URL is required");
            transcribe_youtube(url, media_options(matches, root_matches, interactive)?)
        }
        "pr" => github(matches, settings, Some(GithubKind::Pull), false),
        "issue" => github(matches, settings, Some(GithubKind::Issue), false),
        "comments" => github(matches, settings, None, true),
        _ => Err(usage(
            ErrorCode::UnsupportedInput,
            "unknown ingest command.",
        )),
    }
}

fn github(
    matches: &clap::ArgMatches,
    settings: &crate::ingest::IngestSettings,
    force: Option<GithubKind>,
    always_comments: bool,
) -> Result<IngestedText, CanonicalError> {
    let raw = matches.get_one::<String>("REF").expect("REF is required");
    let mut target = GithubRef::parse(raw)?;
    if let Some(kind) = force {
        target = target.force_kind(kind);
    }
    let comments = always_comments || matches.get_flag("comments");
    fetch_github(&target, comments, settings)
}

fn media_options(
    matches: &clap::ArgMatches,
    root_matches: &clap::ArgMatches,
    interactive: bool,
) -> Result<MediaOptions, CanonicalError> {
    let model = matches
        .get_one::<String>("model")
        .map(String::as_str)
        .and_then(WhisperModel::parse)
        .unwrap_or(WhisperModel::LargeV3Turbo);
    Ok(MediaOptions {
        data_dir: data_dir(root_matches)?,
        model,
        download_model: matches.get_flag("download-model"),
        interactive,
    })
}

fn data_dir(root_matches: &clap::ArgMatches) -> Result<PathBuf, CanonicalError> {
    let mut flags = crate::config::ConfigOverrides::default();
    if let Some(config) = root_matches.get_one::<String>("config") {
        flags = flags.with_config_file(config.clone());
    }
    if let Some(data_dir) = root_matches.get_one::<String>("data-dir") {
        flags = flags.with_data_dir(data_dir.clone());
    }
    let overrides = crate::config::ConfigOverrides::merge(
        flags,
        crate::config::ConfigOverrides::from_environment(),
    );
    let service =
        crate::config::ConfigService::new(&overrides).map_err(crate::cli::detect::config_error)?;
    Ok(service.paths().data_dir().to_path_buf())
}
