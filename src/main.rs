use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(microck_pangram_cli::cli::run())
}
