//! Signed self-update dispatch for `1.0.0` and newer builds.
//!
//! The updater mechanics live in `crate::update`. This module owns only the
//! command-facing policy from `docs/update-contract.md`: which form may
//! prompt, which may mutate, and what each returns. Every path resolves the
//! running executable and its receipt before any network work, so a
//! manager-owned or unowned installation is advised rather than replaced.

// Update is a cold, once-per-invocation boundary that returns the canonical
// error type directly, matching the sibling adapters. Boxing it only to
// satisfy an ABI-size heuristic would add allocation and unwrap noise.
#![allow(clippy::result_large_err)]

use std::io::{IsTerminal as _, Write as _};
use std::path::Path;

use clap::ArgMatches;

use crate::config::{ConfigOverrides, Paths};
use crate::output::{
    CanonicalError, CommandData, ErrorCode, ResolvedCommand, UpdateStatus, UpdateStatusKind,
};
use crate::update::{
    DirectUpdateCandidate, ManagerAdvisory, ReleaseDecision, Target, UpdateCheck, UpdateCheckKind,
    UpdateChecker, UpdateError, cached_availability, detect_manager_install, load_update_state,
    production_manifest_keys, replace_direct_install, require_direct_ownership, store_update_state,
    validate_archive,
};

use super::local_setup::PhaseOneOutcome;

const INSTALL_RECEIPT_FILE_NAME: &str = "install-receipt.json";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Which `pangram update` form the user invoked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateForm {
    /// `--check`: never prompts, never mutates.
    Check,
    /// Bare: prompts on a full TTY, fails closed anywhere else.
    Interactive,
    /// `--yes`: the sole noninteractive install form.
    Assumed,
}

impl UpdateForm {
    const fn command(self) -> ResolvedCommand {
        match self {
            Self::Check => ResolvedCommand::UpdateCheck,
            Self::Interactive | Self::Assumed => ResolvedCommand::UpdateInstall,
        }
    }
}

/// Executes one `update` invocation. Returns the rendered Phase 1 outcome so
/// the caller stays free of updater types.
pub(super) fn execute(root: &ArgMatches, arguments: &ArgMatches) -> PhaseOneOutcome {
    let form = if arguments.get_flag("check") {
        UpdateForm::Check
    } else if arguments.get_flag("yes") {
        UpdateForm::Assumed
    } else {
        UpdateForm::Interactive
    };
    match run(root, form) {
        Ok(outcome) => outcome,
        Err(error) => PhaseOneOutcome::failure(form.command(), error),
    }
}

fn run(root: &ArgMatches, form: UpdateForm) -> Result<PhaseOneOutcome, CanonicalError> {
    let executable = std::env::current_exe().map_err(|_| local_state_error())?;
    let target = Target::current().ok_or_else(unsupported_host_error)?;

    // Manager-owned installations are never mutated. Reporting the manager's
    // own command keeps one owner for the installed bytes.
    if let Some(advisory) = detect_manager_install(&executable) {
        return manager_outcome(form, advisory);
    }

    // The explicit `--data-dir` override wins over the environment, matching
    // every other command that reads local state.
    let mut flags = ConfigOverrides::merge(
        ConfigOverrides::default(),
        ConfigOverrides::from_environment(),
    );
    if let Some(data_dir) = root.get_one::<String>("data-dir") {
        flags = flags.with_data_dir(data_dir.clone());
    }
    let paths = Paths::resolve(&flags).map_err(|_| local_state_error())?;
    let receipt_path = paths.platform_data_dir().join(INSTALL_RECEIPT_FILE_NAME);
    // Ownership is proven before any network request: the receipt must be a
    // protected regular file describing this exact executable, version, and
    // target. Mere existence would let an empty file or a foreign receipt
    // reach the manifest request first.
    require_direct_ownership(&receipt_path, &executable, CURRENT_VERSION, target)
        .map_err(|_| unowned_install_error())?;

    let check = perform_check(paths.data_dir(), target, true)?;
    // A 304 carries no manifest but keeps the previously verified
    // availability. `--check` can answer from that cache; an install needs the
    // manifest itself, so it re-asks without the prior etag.
    let refreshed = match check.kind() {
        UpdateCheckKind::NotModified => {
            match cached_availability(check.state(), CURRENT_VERSION).map_err(update_error)? {
                None => None,
                Some(available) if form == UpdateForm::Check => {
                    commit_state(paths.data_dir(), &check);
                    return success(
                        ResolvedCommand::UpdateCheck,
                        UpdateStatusKind::UpdateAvailable,
                        Some(available),
                        None,
                    );
                }
                Some(_) => Some(perform_check(paths.data_dir(), target, false)?),
            }
        }
        UpdateCheckKind::NoUpdate | UpdateCheckKind::UpdateAvailable => None,
    };
    let check = refreshed.unwrap_or(check);
    let Some(manifest) = check.manifest() else {
        commit_state(paths.data_dir(), &check);
        return success(form.command(), UpdateStatusKind::NoUpdate, None, None);
    };

    let artifact = match manifest
        .release_for(CURRENT_VERSION, CURRENT_VERSION, target)
        .map_err(update_error)?
    {
        ReleaseDecision::NoUpdate => {
            commit_state(paths.data_dir(), &check);
            return success(form.command(), UpdateStatusKind::NoUpdate, None, None);
        }
        ReleaseDecision::Update(artifact) => artifact,
    };
    let available = manifest.version().to_owned();

    if form == UpdateForm::Check {
        commit_state(paths.data_dir(), &check);
        return success(
            ResolvedCommand::UpdateCheck,
            UpdateStatusKind::UpdateAvailable,
            Some(available),
            None,
        );
    }
    if form == UpdateForm::Interactive && !confirm(&available)? {
        // A decline mutates nothing, including updater state, so the next
        // invocation makes the same offer.
        return Ok(PhaseOneOutcome::declined());
    }

    let executable_bytes = download_and_verify(artifact)?;
    let installed_at = crate::domain::UtcTimestamp::now();
    let candidate = DirectUpdateCandidate::new(
        &executable_bytes,
        manifest.version(),
        artifact.sha256(),
        installed_at,
    );
    replace_direct_install(
        &executable,
        &receipt_path,
        CURRENT_VERSION,
        target,
        candidate,
    )
    .map_err(update_error)?;

    // State is committed only after a successful replacement so a failed
    // install cannot suppress the next check.
    commit_state(paths.data_dir(), &check);
    success(
        ResolvedCommand::UpdateInstall,
        UpdateStatusKind::Updated,
        Some(available),
        None,
    )
}

/// Runs one explicit check on a private current-thread runtime. The updater
/// performs no background or ambient checks.
fn perform_check(
    data_dir: &Path,
    target: Target,
    use_cache: bool,
) -> Result<UpdateCheck, CanonicalError> {
    let checker = UpdateChecker::production().map_err(update_error)?;
    // A refresh after a 304 deliberately drops the etag so the response
    // carries the manifest an install needs.
    let prior = if use_cache {
        load_update_state(data_dir).map_err(update_error)?
    } else {
        None
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| local_state_error())?;
    runtime
        .block_on(checker.check(
            prior.as_ref(),
            crate::domain::UtcTimestamp::now(),
            CURRENT_VERSION,
            CURRENT_VERSION,
            target,
            &production_manifest_keys(),
        ))
        .map_err(update_error)
}

fn download_and_verify(
    artifact: &crate::update::UpdateArtifact,
) -> Result<Vec<u8>, CanonicalError> {
    let checker = UpdateChecker::production().map_err(update_error)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| local_state_error())?;
    let archive = runtime
        .block_on(checker.fetch_archive(artifact))
        .map_err(update_error)?;
    validate_archive(artifact, &archive).map_err(update_error)
}

/// Commits verified updater state, ignoring a write failure because the
/// command's result does not depend on the cache.
fn commit_state(data_dir: &Path, check: &UpdateCheck) {
    let _ = store_update_state(data_dir, check.state());
}

/// Asks once on a full TTY. Any redirected stream or `CI` fails closed with
/// `input_required` rather than assuming consent.
fn confirm(available: &str) -> Result<bool, CanonicalError> {
    let interactive = std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal()
        && std::env::var_os("CI").is_none();
    if !interactive {
        return Err(input_required_error());
    }
    let mut stderr = std::io::stderr();
    write!(
        stderr,
        "Install pangram {available} over {CURRENT_VERSION}? [y/N] "
    )
    .and_then(|()| stderr.flush())
    .map_err(|_| local_state_error())?;
    let mut answer = String::new();
    // A read failure or EOF is a decline, never an assumed yes.
    if std::io::stdin().read_line(&mut answer).is_err() {
        return Ok(false);
    }
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "YES"))
}

fn manager_outcome(
    form: UpdateForm,
    advisory: ManagerAdvisory,
) -> Result<PhaseOneOutcome, CanonicalError> {
    success(
        form.command(),
        UpdateStatusKind::NoUpdate,
        None,
        Some(advisory.command().to_owned()),
    )
}

fn success(
    command: ResolvedCommand,
    status: UpdateStatusKind,
    available_version: Option<String>,
    manager_command: Option<String>,
) -> Result<PhaseOneOutcome, CanonicalError> {
    let payload = UpdateStatus::new(status, CURRENT_VERSION, available_version, manager_command)
        .map_err(|_| local_state_error())?;
    let data = match command {
        ResolvedCommand::UpdateCheck => CommandData::UpdateCheck(payload),
        _ => CommandData::UpdateInstall(payload),
    };
    Ok(PhaseOneOutcome::success(data))
}

fn update_error(error: UpdateError) -> CanonicalError {
    error.into_canonical()
}

fn input_required_error() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::InputRequired,
        "Installing an update needs a terminal. Use `pangram update --yes`.",
    )
    .expect("the fixed input-required message is non-empty")
}

fn unowned_install_error() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::UpdateNotOwned,
        "This installation has no direct-install receipt, so it cannot replace itself.",
    )
    .expect("the fixed update-not-owned message is non-empty")
}

fn unsupported_host_error() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::UpdateUnavailable,
        "Updates are unavailable for this host target.",
    )
    .expect("the fixed update-unavailable message is non-empty")
}

fn local_state_error() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::UpdateReplaceFailed,
        "The update could not complete against local state.",
    )
    .expect("the fixed update-replace message is non-empty")
}
