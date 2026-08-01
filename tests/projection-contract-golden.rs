//! Golden generator for the projection fixtures. Regenerate committed
//! fixtures deterministically with:
//!
//! ```text
//! PANGRAM_REGENERATE_GOLDENS=1 \
//!   cargo test --features dev-tools --test projection-contract-golden -- --nocapture
//! ```
//!
//! The fixture module is shared with the contract tests, so several fixture
//! constants are unused inside this particular maintenance target; the
//! file-level allowance contains that single dead-code-by-target case without
//! weakening the actual suite, and the `dev-tools` gate keeps the target out of
//! the default build.

#![allow(dead_code)]

#[path = "support/projection-fixtures.rs"]
mod fixtures;

use microck_pangram_cli::output::{ColorPolicy, CommandEnvelope, OutputFormat, render};

fn render_to(format: OutputFormat, color: ColorPolicy, envelope: &CommandEnvelope) -> String {
    let mut bytes = Vec::new();
    render(format, color, std::slice::from_ref(envelope), &mut bytes).unwrap();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn print_goldens() {
    // Regeneration is opt-in on PANGRAM_REGENERATE_GOLDENS=1 so the
    // `--all-features` CI sweep never rewrites the committed goldens in
    // place; a masked renderer regression would otherwise go unnoticed.
    if std::env::var_os("PANGRAM_REGENERATE_GOLDENS").as_deref() != Some(std::ffi::OsStr::new("1"))
    {
        eprintln!("skipping golden regeneration; set PANGRAM_REGENERATE_GOLDENS=1 to overwrite");
        return;
    }
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/golden");
    let write = |name: &str, content: &str| {
        std::fs::write(root.join(name), content).unwrap();
    };
    write(
        "markdown_success.md",
        &render_to(
            OutputFormat::Markdown,
            ColorPolicy::Plain,
            &fixtures::success_envelope(),
        ),
    );
    write(
        "markdown_failure.md",
        &render_to(
            OutputFormat::Markdown,
            ColorPolicy::Plain,
            &fixtures::failure_envelope(),
        ),
    );
    write(
        "pretty_success.txt",
        &render_to(
            OutputFormat::Pretty,
            ColorPolicy::Plain,
            &fixtures::success_envelope(),
        ),
    );
    write(
        "pretty_failure.txt",
        &render_to(
            OutputFormat::Pretty,
            ColorPolicy::Plain,
            &fixtures::failure_envelope(),
        ),
    );
    write(
        "pretty_success_color.txt",
        &render_to(
            OutputFormat::Pretty,
            ColorPolicy::Color,
            &fixtures::success_envelope(),
        ),
    );
    eprintln!("goldens written to {}", root.display());
}
