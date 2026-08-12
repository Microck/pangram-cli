//! Development-only process driver for compiled loopback acceptance tests.

use std::ffi::OsString;
use std::process::ExitCode;

use microck_pangram_cli::config::ENV_API_KEY;
use microck_pangram_cli::output::ExitCode as PangramExitCode;

fn main() -> ExitCode {
    ExitCode::from(run())
}

fn run() -> u8 {
    let usage = PangramExitCode::Usage.as_u8();
    let mut arguments = std::env::args_os();
    let _program = arguments
        .next()
        .unwrap_or_else(|| OsString::from("pangram-test-driver"));
    let Some(raw_base_url) = arguments.next() else {
        eprintln!("pangram-test-driver: a loopback base URL is required");
        return usage;
    };
    let Some(base_url) = raw_base_url.to_str() else {
        eprintln!("pangram-test-driver: the loopback base URL must be valid UTF-8");
        return usage;
    };
    let api_key = match std::env::var(ENV_API_KEY) {
        Ok(value) if !value.trim().is_empty() => value,
        Ok(_) | Err(std::env::VarError::NotPresent) => {
            eprintln!("pangram-test-driver: PANGRAM_API_KEY is required");
            return usage;
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            eprintln!("pangram-test-driver: PANGRAM_API_KEY must be valid UTF-8");
            return usage;
        }
    };
    // Keep Clap's program name and every rendered usage line identical to the
    // shipped adapter. The driver owns only analyzer construction.
    let forwarded = std::iter::once(OsString::from("pangram")).chain(arguments);
    match microck_pangram_cli::dev_tools::run_with_loopback(base_url, api_key, forwarded) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("pangram-test-driver: {error}");
            usage
        }
    }
}
