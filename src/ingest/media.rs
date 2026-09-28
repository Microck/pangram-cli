//! Local media and YouTube audio transcription.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::analysis::canonical_text_word_count;
use crate::domain::TextOrigin;
use crate::output::{CanonicalError, ErrorCode};

use super::tools::{require_on_path, run};
use super::{IngestedText, MediaOptions, transcription_root, usage, usage_with_recovery};

const HF_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WhisperModel {
    LargeV3Turbo,
    LargeV3TurboQ5,
    LargeV3,
}

impl WhisperModel {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "large-v3-turbo" => Some(Self::LargeV3Turbo),
            "large-v3-turbo-q5_0" => Some(Self::LargeV3TurboQ5),
            "large-v3" => Some(Self::LargeV3),
            _ => None,
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            Self::LargeV3Turbo => "ggml-large-v3-turbo.bin",
            Self::LargeV3TurboQ5 => "ggml-large-v3-turbo-q5_0.bin",
            Self::LargeV3 => "ggml-large-v3.bin",
        }
    }

    fn url(self) -> String {
        format!("{HF_BASE}/{HF_REVISION}/{}", self.file_name())
    }

    fn sha256(self) -> &'static str {
        match self {
            Self::LargeV3Turbo => {
                "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69"
            }
            Self::LargeV3TurboQ5 => {
                "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"
            }
            Self::LargeV3 => "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
        }
    }

    fn size_bytes(self) -> u64 {
        match self {
            Self::LargeV3Turbo => 1_624_555_275,
            Self::LargeV3TurboQ5 => 574_041_195,
            Self::LargeV3 => 3_095_033_483,
        }
    }
}

pub(super) fn transcribe_local(
    path: &Path,
    options: MediaOptions,
) -> Result<IngestedText, CanonicalError> {
    if !path.is_file() {
        return Err(usage(
            ErrorCode::InputRequired,
            "the media path does not exist.",
        ));
    }
    let ffmpeg = require_on_path("ffmpeg", "Install ffmpeg and ensure it is on PATH.")?;
    let whisper = require_whisper(&options)?;
    let model = ensure_model(&options)?;
    let work = work_dir(&options.data_dir)?;
    let wav = work.0.join("audio.wav");
    decode_wav(&ffmpeg, path, &wav)?;
    let text = run_whisper(&whisper, &model, &wav, &work.0)?;
    finish_transcript(text, path.display().to_string())
}

pub(super) fn transcribe_youtube(
    url: &str,
    options: MediaOptions,
) -> Result<IngestedText, CanonicalError> {
    if url.trim().is_empty() {
        return Err(usage(
            ErrorCode::InputRequired,
            "a YouTube URL is required.",
        ));
    }
    let yt_dlp = require_on_path("yt-dlp", "Install yt-dlp and ensure it is on PATH.")?;
    let ffmpeg = require_on_path("ffmpeg", "Install ffmpeg and ensure it is on PATH.")?;
    let whisper = require_whisper(&options)?;
    let model = ensure_model(&options)?;
    let work = work_dir(&options.data_dir)?;
    let audio = work.0.join("audio.wav");
    run(
        &yt_dlp,
        &[
            OsStr::new("--no-playlist"),
            OsStr::new("--no-progress"),
            OsStr::new("-x"),
            OsStr::new("--audio-format"),
            OsStr::new("wav"),
            OsStr::new("-o"),
            audio.as_os_str(),
            OsStr::new(url),
        ],
    )?;
    if !audio.is_file() {
        return Err(usage(
            ErrorCode::TranscriptionFailed,
            "yt-dlp did not produce an audio file.",
        ));
    }
    let wav = work.0.join("mono.wav");
    decode_wav(&ffmpeg, &audio, &wav)?;
    let text = run_whisper(&whisper, &model, &wav, &work.0)?;
    finish_transcript(text, url.to_owned())
}

fn finish_transcript(text: String, name: String) -> Result<IngestedText, CanonicalError> {
    let word_count = canonical_text_word_count(&text);
    if word_count == 0 {
        return Err(usage(
            ErrorCode::TranscriptionFailed,
            "transcription produced no words.",
        ));
    }
    Ok(IngestedText {
        text,
        origin: TextOrigin::Transcript,
        name,
        word_count,
    })
}

fn require_whisper(options: &MediaOptions) -> Result<PathBuf, CanonicalError> {
    let sidecar = transcription_root(&options.data_dir)
        .join("bin")
        .join(whisper_bin_name());
    if sidecar.is_file() {
        return Ok(sidecar);
    }
    require_on_path(
        "whisper-cli",
        "Install whisper.cpp and place whisper-cli on PATH or in the data-dir transcription/bin directory.",
    )
}

fn whisper_bin_name() -> &'static str {
    if cfg!(windows) {
        "whisper-cli.exe"
    } else {
        "whisper-cli"
    }
}

fn ensure_model(options: &MediaOptions) -> Result<PathBuf, CanonicalError> {
    let models = transcription_root(&options.data_dir).join("models");
    let path = models.join(options.model.file_name());
    if path.is_file() {
        if fs::metadata(&path)
            .map(|meta| meta.len() == options.model.size_bytes())
            .unwrap_or(false)
        {
            return Ok(path);
        }
        return Err(usage_with_recovery(
            ErrorCode::TranscriptionFailed,
            "the local Whisper model failed size verification.",
            "Delete the model file and re-run with --download-model.",
        ));
    }
    if options.download_model || (options.interactive && confirm_download(options.model)?) {
        download_model(options.model, &models)?;
        return Ok(models.join(options.model.file_name()));
    }
    Err(usage_with_recovery(
        ErrorCode::ModelDownloadRequired,
        "the local Whisper model is not installed.",
        "Re-run with --download-model to fetch the ggml weights.",
    ))
}

fn confirm_download(model: WhisperModel) -> Result<bool, CanonicalError> {
    let mut stderr = io::stderr().lock();
    writeln!(
        stderr,
        "Download {} to the local transcription directory? [y/N]",
        model.file_name()
    )
    .map_err(|_| {
        usage(
            ErrorCode::InputRequired,
            "could not prompt for model download.",
        )
    })?;
    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line).map_err(|_| {
        usage(
            ErrorCode::InputRequired,
            "could not read the download confirmation.",
        )
    })?;
    let answer = line.trim();
    Ok(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}

fn download_model(model: WhisperModel, models: &Path) -> Result<(), CanonicalError> {
    fs::create_dir_all(models).map_err(|_| {
        usage(
            ErrorCode::InvalidConfig,
            "could not create the transcription model directory.",
        )
    })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| {
            usage(
                ErrorCode::NetworkUnavailable,
                "could not start the local download runtime.",
            )
        })?;
    runtime.block_on(download_model_async(model, models))
}

async fn download_model_async(model: WhisperModel, models: &Path) -> Result<(), CanonicalError> {
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .build()
        .map_err(|_| {
            usage(
                ErrorCode::NetworkUnavailable,
                "could not build the download client.",
            )
        })?;
    let response = client
        .get(model.url())
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|_| {
            usage(
                ErrorCode::NetworkUnavailable,
                "the Whisper model download failed.",
            )
        })?;
    let bytes = response.bytes().await.map_err(|_| {
        usage(
            ErrorCode::NetworkUnavailable,
            "the Whisper model download failed.",
        )
    })?;
    if bytes.len() as u64 != model.size_bytes() || hex_sha256(&bytes) != model.sha256() {
        return Err(usage(
            ErrorCode::TranscriptionFailed,
            "the downloaded Whisper model failed SHA-256 verification.",
        ));
    }
    let dest = models.join(model.file_name());
    let tmp = dest.with_extension("bin.part");
    fs::write(&tmp, &bytes).map_err(|_| {
        usage(
            ErrorCode::InvalidConfig,
            "could not write the Whisper model.",
        )
    })?;
    fs::rename(&tmp, &dest).map_err(|_| {
        usage(
            ErrorCode::InvalidConfig,
            "could not install the Whisper model.",
        )
    })?;
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_wav(ffmpeg: &Path, input: &Path, output: &Path) -> Result<(), CanonicalError> {
    run(
        ffmpeg,
        &[
            OsStr::new("-nostdin"),
            OsStr::new("-hide_banner"),
            OsStr::new("-loglevel"),
            OsStr::new("error"),
            OsStr::new("-y"),
            OsStr::new("-i"),
            input.as_os_str(),
            OsStr::new("-ar"),
            OsStr::new("16000"),
            OsStr::new("-ac"),
            OsStr::new("1"),
            OsStr::new("-c:a"),
            OsStr::new("pcm_s16le"),
            output.as_os_str(),
        ],
    )?;
    Ok(())
}

fn run_whisper(
    whisper: &Path,
    model: &Path,
    wav: &Path,
    work: &Path,
) -> Result<String, CanonicalError> {
    let prefix = work.join("transcript");
    run(
        whisper,
        &[
            OsStr::new("-m"),
            model.as_os_str(),
            OsStr::new("-f"),
            wav.as_os_str(),
            OsStr::new("-otxt"),
            OsStr::new("-of"),
            prefix.as_os_str(),
        ],
    )?;
    let text = fs::read_to_string(prefix.with_extension("txt")).map_err(|_| {
        usage(
            ErrorCode::TranscriptionFailed,
            "whisper-cli did not write a transcript.",
        )
    })?;
    Ok(text)
}

struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn work_dir(data_dir: &Path) -> Result<WorkDir, CanonicalError> {
    let dir = transcription_root(data_dir)
        .join("work")
        .join(uuid::Uuid::now_v7().to_string());
    fs::create_dir_all(&dir).map_err(|_| {
        usage(
            ErrorCode::InvalidConfig,
            "could not create a transcription work directory.",
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|_| {
            usage(
                ErrorCode::InvalidConfig,
                "could not restrict the transcription work directory.",
            )
        })?;
    }
    Ok(WorkDir(dir))
}
