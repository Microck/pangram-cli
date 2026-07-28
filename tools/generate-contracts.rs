use std::path::Path;

use microck_pangram_cli::contracts::write_generated_artifacts;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    write_generated_artifacts(Path::new(env!("CARGO_MANIFEST_DIR")))?;
    Ok(())
}
