use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::ExitCode,
};

const LINE_WARNING_THRESHOLD: usize = 800;
const LINE_ERROR_THRESHOLD: usize = 1_000;
// The roadmap mandates a file-size CI gate without fixing a number, so this
// conservative 1 MiB ceiling is internal hygiene policy. The cap reads file
// metadata before content so an oversized one-line file cannot bypass the
// line limit through a large unvalidated allocation.
const MAX_TEXT_FILE_BYTES: u64 = 1_048_576; // 1 MiB
// ADR 0008 grants the generated output-schema union a line-limit exception;
// ADR 0009 grants the single normative contracts reference one. Both documents
// still receive every other hygiene check.
const LINE_LIMIT_EXCEPTIONS: &[&str] = &["contracts/output.schema.json", "docs/contracts.md"];
// These are exact paths relative to the repository root, not directory basenames.
const EXCLUDED_ROOT_DIRECTORIES: &[&str] = &[
    ".codebase-memory",
    ".git",
    ".jj",
    ".next",
    "coverage",
    "dist",
    "node_modules",
    "target",
];
const EXCLUDED_LOCKFILES: &[&str] = &[
    // Dependency lockfiles are generated inventories, not source, docs, or config.
    "Cargo.lock",
    "bun.lock",
    "bun.lockb",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
];
const BINARY_EXTENSIONS: &[&str] = &[
    "7z", "a", "avi", "bin", "bmp", "bz2", "class", "dylib", "eot", "exe", "gif", "gz", "ico",
    "jar", "jpeg", "jpg", "mov", "mp3", "mp4", "o", "otf", "pdf", "png", "so", "tar", "tgz", "tif",
    "tiff", "ttf", "wasm", "webm", "webp", "woff", "woff2", "xz", "zip", "zst",
];
const TEXT_EXTENSIONS: &[&str] = &[
    "cjs", "css", "graphql", "html", "js", "json", "jsx", "md", "mjs", "ps1", "rs", "scss", "sh",
    "snap", "sql", "toml", "ts", "tsx", "txt", "yaml", "yml",
];
const EXTENSIONLESS_TEXT_FILES: &[&str] = &[
    ".editorconfig",
    ".gitattributes",
    ".gitignore",
    ".npmrc",
    ".prettierignore",
    "Dockerfile",
    "LICENSE",
    "Makefile",
];

fn main() -> ExitCode {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    let root = arguments
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    if arguments.next().is_some() {
        eprintln!("usage: check-hygiene [repository-root]");
        return ExitCode::FAILURE;
    }

    match check_repository(&root) {
        Ok(file_count) => {
            println!("hygiene check passed for {file_count} files");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn check_repository(root: &Path) -> Result<usize, String> {
    if !root.is_dir() {
        return Err(format!(
            "repository root is not a directory: {}",
            root.display()
        ));
    }

    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by_key(|path| normalized_relative_path(root, path));

    let mut scanned_files = 0;
    let mut violations = 0;

    for path in &files {
        let relative_path = normalized_relative_path(root, path);
        let Some(contents) = read_text_file(path, &relative_path)? else {
            continue;
        };
        scanned_files += 1;
        let line_count = contents.lines().count();
        let has_line_limit_exception = LINE_LIMIT_EXCEPTIONS.contains(&relative_path.as_str());

        if line_count >= LINE_WARNING_THRESHOLD && !has_line_limit_exception {
            eprintln!(
                "warning: {relative_path} has {line_count} lines; review decomposition at \
                 {LINE_WARNING_THRESHOLD} lines"
            );
        }

        if line_count > LINE_ERROR_THRESHOLD && !has_line_limit_exception {
            eprintln!(
                "error: {relative_path} has {line_count} lines; the limit is \
                 {LINE_ERROR_THRESHOLD}"
            );
            violations += 1;
        }

        for (line_index, line) in contents.split('\n').enumerate() {
            for (column_index, character) in line.chars().enumerate() {
                if let Some(name) = forbidden_character_name(character) {
                    eprintln!(
                        "error: {relative_path}:{}:{} contains {name}",
                        line_index + 1,
                        column_index + 1
                    );
                    violations += 1;
                }
            }
        }
    }

    if violations == 0 {
        Ok(scanned_files)
    } else {
        Err(format!("{violations} hygiene violation(s) found"))
    }
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read directory {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot read directory entry: {error}"))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;

        if file_type.is_symlink() {
            return Err(format!(
                "symbolic links are not allowed: {}",
                normalized_relative_path(root, &path)
            ));
        } else if file_type.is_dir() {
            if !is_excluded_directory(root, &path) {
                collect_files(root, &path, files)?;
            }
        } else if file_type.is_file() && !is_excluded_file(&path) {
            files.push(path);
        }
    }

    Ok(())
}

fn is_excluded_directory(root: &Path, path: &Path) -> bool {
    let Ok(relative_path) = path.strip_prefix(root) else {
        return false;
    };

    EXCLUDED_ROOT_DIRECTORIES
        .iter()
        .any(|excluded| relative_path == Path::new(excluded))
}

fn is_excluded_file(path: &Path) -> bool {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| EXCLUDED_LOCKFILES.contains(&name))
    {
        return true;
    }

    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            BINARY_EXTENSIONS
                .iter()
                .any(|binary| extension.eq_ignore_ascii_case(binary))
        })
}

fn read_text_file(path: &Path, relative_path: &str) -> Result<Option<String>, String> {
    let expected_text = is_expected_text_file(path);
    classify_text_source(
        path,
        relative_path,
        expected_text,
        file_byte_length,
        file_contents,
    )
}

fn file_byte_length(path: &Path) -> io::Result<u64> {
    fs::metadata(path).map(|metadata| metadata.len())
}

fn file_contents(path: &Path) -> io::Result<Vec<u8>> {
    fs::read(path)
}

/// Classifies one candidate file through metadata-then-content probes.
///
/// The size gate consults only metadata (a byte length) and returns before
/// any content read, so an over-limit file is rejected without an unvalidated
/// allocation. The probes are parameters so tests can prove that over-limit
/// files never reach the read probe.
fn classify_text_source(
    path: &Path,
    relative_path: &str,
    expected_text: bool,
    byte_length: fn(&Path) -> io::Result<u64>,
    read: fn(&Path) -> io::Result<Vec<u8>>,
) -> Result<Option<String>, String> {
    let byte_len =
        byte_length(path).map_err(|error| format!("cannot stat {relative_path}: {error}"))?;
    if byte_len > MAX_TEXT_FILE_BYTES {
        return if expected_text {
            Err(format!(
                "{relative_path} has {byte_len} bytes; the text-file limit is \
                 {MAX_TEXT_FILE_BYTES} bytes"
            ))
        } else {
            Ok(None)
        };
    }

    let bytes = read(path).map_err(|error| format!("cannot read {relative_path}: {error}"))?;

    if bytes.contains(&0) {
        return if expected_text {
            Err(format!("{relative_path} contains NUL bytes"))
        } else {
            Ok(None)
        };
    }

    match String::from_utf8(bytes) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if expected_text => Err(format!(
            "cannot read {relative_path} as UTF-8 text: {error}"
        )),
        Err(_) => Ok(None),
    }
}

fn is_expected_text_file(path: &Path) -> bool {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| EXTENSIONLESS_TEXT_FILES.contains(&name))
    {
        return true;
    }

    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            TEXT_EXTENSIONS
                .iter()
                .any(|text| extension.eq_ignore_ascii_case(text))
        })
}

fn normalized_relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn forbidden_character_name(character: char) -> Option<&'static str> {
    // Keep this list aligned with AGENTS.md. Other Unicode remains valid.
    match character {
        '\u{00b7}' => Some("middle dot (U+00B7)"),
        '\u{2011}' => Some("non-breaking hyphen (U+2011)"),
        '\u{2013}' => Some("en dash (U+2013)"),
        '\u{2014}' => Some("em dash (U+2014)"),
        '\u{2018}' => Some("left single quotation mark (U+2018)"),
        '\u{2019}' => Some("right single quotation mark (U+2019)"),
        '\u{201c}' => Some("left double quotation mark (U+201C)"),
        '\u{201d}' => Some("right double quotation mark (U+201D)"),
        '\u{2022}' => Some("bullet (U+2022)"),
        '\u{feff}' => Some("byte order mark (U+FEFF)"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::{Path, PathBuf},
        process,
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        MAX_TEXT_FILE_BYTES, check_repository, classify_text_source, forbidden_character_name,
    };

    static NEXT_TEMP_DIRECTORY: AtomicUsize = AtomicUsize::new(0);
    static READ_PROBE_CALLED: AtomicUsize = AtomicUsize::new(0);

    fn over_limit_byte_length(_path: &Path) -> std::io::Result<u64> {
        Ok(MAX_TEXT_FILE_BYTES + 1)
    }

    fn at_limit_byte_length(_path: &Path) -> std::io::Result<u64> {
        Ok(MAX_TEXT_FILE_BYTES)
    }

    fn counting_read(_path: &Path) -> std::io::Result<Vec<u8>> {
        READ_PROBE_CALLED.fetch_add(1, Ordering::Relaxed);
        Ok(Vec::new())
    }

    fn limit_read(_path: &Path) -> std::io::Result<Vec<u8>> {
        Ok(b"within the limit\n".to_vec())
    }

    struct TempDirectory {
        path: PathBuf,
    }

    impl TempDirectory {
        fn new() -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "pangram-hygiene-{}-{timestamp}-{sequence}",
                process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn rejects_each_documented_forbidden_character() {
        let forbidden = [
            '\u{00b7}', '\u{2011}', '\u{2013}', '\u{2014}', '\u{2018}', '\u{2019}', '\u{201c}',
            '\u{201d}', '\u{2022}', '\u{feff}',
        ];

        for character in forbidden {
            assert!(forbidden_character_name(character).is_some());
        }
    }

    #[test]
    fn permits_unicode_that_is_not_forbidden() {
        let allowed = ['\u{00e9}', '\u{2026}', '\u{2192}', '\u{4e2d}', '\u{1f642}'];

        for character in allowed {
            assert!(forbidden_character_name(character).is_none());
        }
    }

    #[test]
    fn scans_extensionless_text_files() {
        let repository = TempDirectory::new();
        fs::write(repository.path().join(".gitignore"), "target/\n").unwrap();
        fs::write(repository.path().join("LICENSE"), "Permission granted.\n").unwrap();

        assert_eq!(check_repository(repository.path()).unwrap(), 2);

        fs::write(
            repository.path().join("LICENSE"),
            "forbidden \u{2014} punctuation\n",
        )
        .unwrap();
        assert!(check_repository(repository.path()).is_err());
    }

    #[test]
    fn skips_unknown_binary_files() {
        let repository = TempDirectory::new();
        fs::write(
            repository.path().join("asset.blob"),
            [0x00, 0x9f, 0x92, 0x96, 0xff],
        )
        .unwrap();

        assert_eq!(check_repository(repository.path()).unwrap(), 0);
    }

    // The roadmap mandates a file-size CI gate (c); it must reject an
    // over-limit expected-text file using metadata alone, without reading
    // content.
    #[test]
    fn rejects_oversized_expected_text_files_using_metadata_before_any_read() {
        READ_PROBE_CALLED.store(0, Ordering::Relaxed);
        let result = classify_text_source(
            Path::new("huge.md"),
            "huge.md",
            true,
            over_limit_byte_length,
            counting_read,
        );

        assert_eq!(
            READ_PROBE_CALLED.load(Ordering::Relaxed),
            0,
            "over-limit files must not be read"
        );
        let error = result.unwrap_err();
        assert!(error.contains("huge.md has 1048577 bytes"), "{error}");
        assert!(
            error.contains("the text-file limit is 1048576 bytes"),
            "{error}"
        );
    }

    #[test]
    fn skips_oversized_unknown_extensions_using_metadata_before_any_read() {
        READ_PROBE_CALLED.store(0, Ordering::Relaxed);
        let result = classify_text_source(
            Path::new("huge.blob"),
            "huge.blob",
            false,
            over_limit_byte_length,
            counting_read,
        );

        assert_eq!(
            READ_PROBE_CALLED.load(Ordering::Relaxed),
            0,
            "over-limit files must not be read"
        );
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn admits_files_at_the_size_limit() {
        let result = classify_text_source(
            Path::new("ok.md"),
            "ok.md",
            true,
            at_limit_byte_length,
            limit_read,
        );

        assert_eq!(result.unwrap().as_deref(), Some("within the limit\n"));
    }

    #[test]
    fn rejects_an_end_to_end_over_limit_expected_text_file() {
        let repository = TempDirectory::new();
        let oversized = vec![b'x'; (MAX_TEXT_FILE_BYTES + 1) as usize];
        fs::write(repository.path().join("huge.md"), oversized).unwrap();

        let error = check_repository(repository.path()).unwrap_err();
        assert!(error.contains("huge.md"), "{error}");
    }

    #[test]
    fn accepts_an_end_to_end_expected_text_file_at_the_under_limit() {
        let repository = TempDirectory::new();
        fs::write(repository.path().join("ok.md"), "small\n").unwrap();

        assert_eq!(check_repository(repository.path()).unwrap(), 1);
    }

    #[test]
    fn excludes_generated_directories_only_at_repository_root() {
        let repository = TempDirectory::new();
        let root_target = repository.path().join("target");
        fs::create_dir(&root_target).unwrap();
        fs::write(
            root_target.join("example.md"),
            "ignored \u{2014} generated output\n",
        )
        .unwrap();

        assert_eq!(check_repository(repository.path()).unwrap(), 0);

        let nested_target = repository.path().join("docs/target");
        fs::create_dir_all(&nested_target).unwrap();
        fs::write(
            nested_target.join("example.md"),
            "checked \u{2014} documentation\n",
        )
        .unwrap();

        assert!(check_repository(repository.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_links() {
        use std::os::unix::fs::symlink;

        let sandbox = TempDirectory::new();
        let repository = sandbox.path().join("repository");
        fs::create_dir(&repository).unwrap();
        let outside = sandbox.path().join("outside.md");
        fs::write(&outside, "outside\n").unwrap();
        symlink(outside, repository.join("linked.md")).unwrap();

        let error = check_repository(&repository).unwrap_err();
        assert!(
            error.contains("symbolic links are not allowed: linked.md"),
            "{error}"
        );
    }
}
