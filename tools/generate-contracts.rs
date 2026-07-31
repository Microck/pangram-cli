use std::path::PathBuf;

use microck_pangram_cli::contracts::write_generated_artifacts;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Resolve the repository root at runtime rather than baking the
    // build-time manifest path into the binary. `env!("CARGO_MANIFEST_DIR")`
    // is fixed at compile time, so a shared or cached `target/` directory
    // across worktrees would silently regenerate the wrong checkout. Prefer
    // an explicit argument, then the runtime variable, then the current dir.
    let root = match std::env::args_os().nth(1) {
        Some(argument) => PathBuf::from(argument),
        None => std::env::var_os("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .ok_or("pass the repository root, or run via `cargo run`")?,
    };
    write_generated_artifacts(&root)?;
    Ok(())
}
