use std::fs;
use std::io::{Cursor, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use tar::{Builder, Header};
use xz2::write::XzEncoder;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use microck_pangram_cli::domain::Sha256Hash;

const SHELLS: [(&str, &str); 5] = [
    ("bash", "pangram.bash"),
    ("zsh", "_pangram"),
    ("fish", "pangram.fish"),
    ("powershell", "pangram.ps1"),
    ("elvish", "pangram.elv"),
];

#[derive(Serialize)]
struct ArtifactMetadata {
    target: String,
    archive_format: &'static str,
    file_name: String,
    size_bytes: u64,
    executable_size_bytes: u64,
    sha256: Sha256Hash,
}

fn main() {
    if let Err(message) = run() {
        eprintln!("package-release: {message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let mut target = None;
    let mut executable = None;
    let mut output = None;
    while let Some(argument) = arguments.next() {
        let value = arguments
            .next()
            .ok_or_else(|| format!("{argument} requires a value"))?;
        match argument.as_str() {
            "--target" => target = Some(value),
            "--executable" => executable = Some(PathBuf::from(value)),
            "--out-dir" => output = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown argument {argument}")),
        }
    }
    let target = target.ok_or("--target is required")?;
    let executable = executable.ok_or("--executable is required")?;
    let output = output.ok_or("--out-dir is required")?;
    let windows = target == "x86_64-pc-windows-msvc";
    if !matches!(
        target.as_str(),
        "x86_64-unknown-linux-gnu"
            | "aarch64-unknown-linux-gnu"
            | "x86_64-apple-darwin"
            | "aarch64-apple-darwin"
            | "x86_64-pc-windows-msvc"
    ) {
        return Err("unsupported release target".into());
    }
    let executable_bytes = fs::read(&executable).map_err(|_| "cannot read executable")?;
    if executable_bytes.is_empty() {
        return Err("executable is empty".into());
    }
    let readme = fs::read("README.md").map_err(|_| "cannot read README.md")?;
    let license = fs::read("LICENSE").map_err(|_| "cannot read LICENSE")?;
    let completions = generate_completions(&executable)?;
    let man_page = generate_man_page(&executable)?;

    fs::create_dir_all(&output).map_err(|_| "cannot create output directory")?;
    let extension = if windows { "zip" } else { "tar.xz" };
    let file_name = format!(
        "pangram-v{}-{target}.{extension}",
        env!("CARGO_PKG_VERSION")
    );
    let archive_path = output.join(&file_name);
    let archive_bytes = if windows {
        build_zip(
            &executable_bytes,
            &readme,
            &license,
            &completions,
            &man_page,
        )?
    } else {
        build_tar_xz(
            &executable_bytes,
            &readme,
            &license,
            &completions,
            &man_page,
        )?
    };
    fs::write(&archive_path, &archive_bytes).map_err(|_| "cannot write archive")?;

    let metadata = ArtifactMetadata {
        target: target.clone(),
        archive_format: extension,
        file_name,
        size_bytes: archive_bytes.len() as u64,
        executable_size_bytes: executable_bytes.len() as u64,
        sha256: Sha256Hash::digest(&archive_bytes),
    };
    let mut metadata_bytes =
        serde_json::to_vec_pretty(&metadata).map_err(|_| "cannot encode metadata")?;
    metadata_bytes.push(b'\n');
    fs::write(
        output.join(format!("{target}.artifact.json")),
        metadata_bytes,
    )
    .map_err(|_| "cannot write artifact metadata")?;
    Ok(())
}

fn generate_completions(executable: &Path) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    SHELLS
        .iter()
        .map(|(shell, name)| {
            let output = Command::new(executable)
                .args(["completions", shell])
                .output()
                .map_err(|_| "cannot run completion generator")?;
            if !output.status.success() || output.stdout.is_empty() {
                return Err("completion generation failed".into());
            }
            Ok((*name, output.stdout))
        })
        .collect()
}

fn generate_man_page(executable: &Path) -> Result<Vec<u8>, String> {
    let output = Command::new(executable)
        .arg("--help")
        .output()
        .map_err(|_| "cannot run help generator")?;
    if !output.status.success() {
        return Err("help generation failed".into());
    }
    let help = String::from_utf8(output.stdout).map_err(|_| "help is not UTF-8")?;
    let escaped = help.replace('\\', "\\e").replace('-', "\\-");
    Ok(format!(
        ".TH PANGRAM 1\n.SH NAME\npangram \\- unofficial Pangram terminal client\n.SH SYNOPSIS\n.nf\n{escaped}.fi\n"
    )
    .into_bytes())
}

fn build_tar_xz(
    executable: &[u8],
    readme: &[u8],
    license: &[u8],
    completions: &[(&str, Vec<u8>)],
    man_page: &[u8],
) -> Result<Vec<u8>, String> {
    // Level 6 keeps release artifacts compact without making clean-machine
    // packaging disproportionately CPU-heavy.
    let encoder = XzEncoder::new(Vec::new(), 6);
    let mut archive = Builder::new(encoder);
    append_tar(&mut archive, "pangram", executable, 0o755)?;
    append_tar(&mut archive, "README.md", readme, 0o644)?;
    append_tar(&mut archive, "LICENSE", license, 0o644)?;
    for (name, bytes) in completions {
        append_tar(&mut archive, &format!("completions/{name}"), bytes, 0o644)?;
    }
    append_tar(&mut archive, "man/pangram.1", man_page, 0o644)?;
    archive
        .into_inner()
        .map_err(|_| "cannot finish tar archive")?
        .finish()
        .map_err(|_| "cannot finish xz archive".into())
}

fn append_tar(
    archive: &mut Builder<XzEncoder<Vec<u8>>>,
    path: &str,
    bytes: &[u8],
    mode: u32,
) -> Result<(), String> {
    let mut header = Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    archive
        .append_data(&mut header, path, bytes)
        .map_err(|_| "cannot append tar entry".into())
}

fn build_zip(
    executable: &[u8],
    readme: &[u8],
    license: &[u8],
    completions: &[(&str, Vec<u8>)],
    man_page: &[u8],
) -> Result<Vec<u8>, String> {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    append_zip(&mut archive, "pangram.exe", executable, 0o755)?;
    append_zip(&mut archive, "README.md", readme, 0o644)?;
    append_zip(&mut archive, "LICENSE", license, 0o644)?;
    for (name, bytes) in completions {
        append_zip(&mut archive, &format!("completions/{name}"), bytes, 0o644)?;
    }
    append_zip(&mut archive, "man/pangram.1", man_page, 0o644)?;
    archive
        .finish()
        .map_err(|_| "cannot finish zip archive".into())
        .map(Cursor::into_inner)
}

fn append_zip(
    archive: &mut ZipWriter<Cursor<Vec<u8>>>,
    path: &str,
    bytes: &[u8],
    mode: u32,
) -> Result<(), String> {
    let options = SimpleFileOptions::default().unix_permissions(mode);
    archive
        .start_file(path, options)
        .map_err(|_| "cannot append zip entry")?;
    archive
        .write_all(bytes)
        .map_err(|_| "cannot write zip entry".into())
}
