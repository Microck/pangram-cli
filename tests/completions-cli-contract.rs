//! Compiled-binary contracts for the raw shell-completion surface.

use microck_pangram_cli::cli::{Availability, try_run_from};

#[allow(dead_code)]
#[path = "support/cli_contract_env.rs"]
mod harness;

use harness::{argument, command, pangram};

const SHELLS: &[(&str, &str)] = &[
    ("bash", "_pangram()"),
    ("zsh", "#compdef pangram"),
    ("fish", "complete -c pangram"),
    (
        "powershell",
        "Register-ArgumentCompleter -Native -CommandName 'pangram'",
    ),
    ("elvish", "set edit:completion:arg-completer[pangram]"),
];

#[test]
fn every_contracted_shell_emits_only_its_completion_script() {
    for (shell, identifying_source) in SHELLS {
        let output = pangram()
            .args(["completions", shell])
            .output()
            .expect("run the compiled completion command");

        assert_eq!(output.status.code(), Some(0), "{shell}");
        assert!(output.stderr.is_empty(), "{shell} emitted diagnostics");
        assert!(!output.stdout.is_empty(), "{shell} emitted no script");
        let script = String::from_utf8(output.stdout).expect("completion scripts are UTF-8");
        assert!(
            script.contains(identifying_source),
            "{shell} output is not the requested completion script:\n{script}"
        );
    }
}

#[test]
fn shell_vocabulary_is_exact_and_case_sensitive() {
    for rejected in ["Bash", "power-shell", "pwsh", "nu", ""] {
        let mut invocation = vec!["completions"];
        if !rejected.is_empty() {
            invocation.push(rejected);
        }
        let output = pangram()
            .args(invocation)
            .output()
            .expect("run the compiled completion command");

        assert_eq!(output.status.code(), Some(2), "{rejected:?}");
        assert!(output.stdout.is_empty(), "{rejected:?}");
        let stderr = String::from_utf8(output.stderr).expect("usage errors are UTF-8");
        if rejected.is_empty() {
            assert!(stderr.contains("Usage:"), "missing shell: {stderr}");
        } else {
            assert!(
                stderr.contains("possible values: bash, elvish, fish, powershell, zsh"),
                "{rejected:?}: {stderr}"
            );
        }
    }
}

#[test]
fn completions_and_its_shell_argument_are_available_together() {
    let completions = command(&["completions"]);
    let shell = argument(completions, "SHELL");

    assert_eq!(completions.availability, Availability::Available);
    assert_eq!(shell.availability, Availability::Available);
    assert!(shell.required);
    assert_eq!(
        shell.accepted_values,
        &["bash", "zsh", "fish", "powershell", "elvish"]
    );
}

#[test]
fn library_parsing_accepts_completions_without_owning_process_output() {
    // The library hook is a parser, not a renderer. The process-facing test
    // above proves output; this call proves the same command remains safe for
    // callers which only validate argv in-process.
    try_run_from(["pangram", "completions", "bash"]).unwrap();
}
