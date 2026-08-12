pub mod analysis;
pub mod cli;
pub mod config;
pub mod contracts;
pub mod diagnostics;
pub mod domain;
pub mod history;
pub mod output;
pub(crate) mod tui;

/// Development-only compiled adapter entry points. These are not a stable
/// Rust interface and are absent from normal builds and release artifacts.
#[cfg(feature = "dev-tools")]
#[doc(hidden)]
pub mod dev_tools {
    use std::ffi::OsString;

    use secrecy::SecretString;

    /// Constructs a loopback-only analyzer, then enters the exact same
    /// process-facing CLI or TUI dispatch used by the shipped binary.
    pub fn run_with_loopback<I, T>(
        base_url: &str,
        api_key: String,
        arguments: I,
    ) -> Result<u8, String>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let analyzer =
            crate::analysis::build_loopback_analyzer(base_url, SecretString::from(api_key))
                .map_err(|error| error.message().to_owned())?;
        Ok(crate::cli::run_with_analyzer(arguments, analyzer))
    }
}
