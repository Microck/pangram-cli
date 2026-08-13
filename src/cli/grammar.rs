use serde::Serialize;

/// The roadmap phase that first owns a command or argument.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Phase {
    Scaffold = 0,
    LocalSetup = 1,
    TextDetection = 2,
    BulkAndTasks = 3,
    History = 4,
    Tui = 5,
    McpAndAgents = 6,
    FileAndPlagiarism = 7,
    DistributionAndUpdate = 8,
}

impl Serialize for Phase {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u8(*self as u8)
    }
}

/// Whether the current binary accepts the referenced surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    Planned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    Entrypoint,
    Namespace,
    Command,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentKind {
    Positional,
    Option,
    Flag,
}

/// One argument in the complete planned grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ArgumentSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub kind: ArgumentKind,
    pub value_name: Option<&'static str>,
    pub accepted_values: &'static [&'static str],
    pub required: bool,
    pub repeatable: bool,
    pub group: Option<&'static str>,
    pub requires: &'static [&'static str],
    pub stdin_marker: Option<&'static str>,
    pub phase: Phase,
    pub availability: Availability,
}

impl ArgumentSpec {
    const fn with_aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }

    const fn with_value_name(mut self, value_name: &'static str) -> Self {
        self.value_name = Some(value_name);
        self
    }

    const fn with_accepted_values(mut self, accepted_values: &'static [&'static str]) -> Self {
        self.accepted_values = accepted_values;
        self
    }

    const fn required(mut self) -> Self {
        self.required = true;
        self
    }

    const fn repeatable(mut self) -> Self {
        self.repeatable = true;
        self
    }

    const fn in_group(mut self, group: &'static str) -> Self {
        self.group = Some(group);
        self
    }

    const fn requiring(mut self, required_arguments: &'static [&'static str]) -> Self {
        self.requires = required_arguments;
        self
    }

    const fn with_stdin_marker(mut self, stdin_marker: &'static str) -> Self {
        self.stdin_marker = Some(stdin_marker);
        self
    }

    const fn available(mut self) -> Self {
        self.availability = Availability::Available;
        self
    }
}

/// A group adds a relationship that individual argument rows cannot express.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ArgumentGroupSpec {
    pub name: &'static str,
    pub required: bool,
    pub exclusive: bool,
    pub implicit_members: &'static [&'static str],
}

/// One command path in the complete planned grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CommandSpec {
    pub path: &'static [&'static str],
    pub kind: CommandKind,
    pub arguments: &'static [ArgumentSpec],
    pub argument_groups: &'static [ArgumentGroupSpec],
    pub phase: Phase,
    pub availability: Availability,
}

/// Ordered generator input for the complete CLI reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct GrammarSpec {
    pub name: &'static str,
    pub global_arguments: &'static [ArgumentSpec],
    pub commands: &'static [CommandSpec],
}

const OUTPUT_FORMATS: &[&str] = &["json", "jsonl", "toon", "markdown", "pretty"];
const PROGRESS_MODES: &[&str] = &["auto", "never", "jsonl"];
const MCP_CLIENTS: &[&str] = &[
    "claude-code",
    "claude-desktop",
    "codex",
    "cursor",
    "vscode",
    "windsurf",
    "gemini",
    "opencode",
    "cline",
    "roo-code",
    "droid",
    "antigravity",
];
const SHELLS: &[&str] = &["bash", "zsh", "fish", "powershell", "elvish"];

const fn planned_argument(name: &'static str, kind: ArgumentKind, phase: Phase) -> ArgumentSpec {
    ArgumentSpec {
        name,
        aliases: &[],
        kind,
        value_name: None,
        accepted_values: &[],
        required: false,
        repeatable: false,
        group: None,
        requires: &[],
        stdin_marker: None,
        phase,
        availability: Availability::Planned,
    }
}

const fn positional(
    name: &'static str,
    accepted_values: &'static [&'static str],
    required: bool,
    repeatable: bool,
    phase: Phase,
) -> ArgumentSpec {
    let mut argument = planned_argument(name, ArgumentKind::Positional, phase)
        .with_value_name(name)
        .with_accepted_values(accepted_values);
    if required {
        argument = argument.required();
    }
    if repeatable {
        argument = argument.repeatable();
    }
    argument
}

const fn option(
    name: &'static str,
    value_name: &'static str,
    accepted_values: &'static [&'static str],
    required: bool,
    repeatable: bool,
    phase: Phase,
) -> ArgumentSpec {
    let mut argument = planned_argument(name, ArgumentKind::Option, phase)
        .with_value_name(value_name)
        .with_accepted_values(accepted_values);
    if required {
        argument = argument.required();
    }
    if repeatable {
        argument = argument.repeatable();
    }
    argument
}

const fn flag(name: &'static str, phase: Phase) -> ArgumentSpec {
    planned_argument(name, ArgumentKind::Flag, phase)
}

const fn grouped_argument(
    name: &'static str,
    kind: ArgumentKind,
    value_name: Option<&'static str>,
    group: &'static str,
    phase: Phase,
) -> ArgumentSpec {
    let argument = planned_argument(name, kind, phase).in_group(group);
    match value_name {
        Some(value_name) => argument.with_value_name(value_name),
        None => argument,
    }
}

const fn planned_command(
    path: &'static [&'static str],
    kind: CommandKind,
    arguments: &'static [ArgumentSpec],
    argument_groups: &'static [ArgumentGroupSpec],
    phase: Phase,
) -> CommandSpec {
    CommandSpec {
        path,
        kind,
        arguments,
        argument_groups,
        phase,
        availability: Availability::Planned,
    }
}

/// A command whose compiled behavior and contract tests landed together.
const fn available_command(
    path: &'static [&'static str],
    kind: CommandKind,
    arguments: &'static [ArgumentSpec],
    argument_groups: &'static [ArgumentGroupSpec],
    phase: Phase,
) -> CommandSpec {
    CommandSpec {
        path,
        kind,
        arguments,
        argument_groups,
        phase,
        availability: Availability::Available,
    }
}

#[rustfmt::skip]
const GLOBAL_ARGUMENTS: &[ArgumentSpec] = &[
    option("--config", "PATH", &[], false, false, Phase::LocalSetup).available(),
    option("--data-dir", "PATH", &[], false, false, Phase::LocalSetup).available(),
    option("--error-format", "FORMAT", &["json", "text"], false, false, Phase::TextDetection).available(),
    flag("--no-color", Phase::TextDetection).available(),
    flag("--version", Phase::Scaffold).with_aliases(&["-V"]).available(),
    flag("--help", Phase::Scaffold).with_aliases(&["-h"]).available(),
];

#[rustfmt::skip]
const ROOT_ARGUMENTS: &[ArgumentSpec] = &[
    positional("TEXT", &[], false, false, Phase::TextDetection).in_group("source_category").with_stdin_marker("-").available(),
];

const SOURCE_CATEGORY_GROUP: &[ArgumentGroupSpec] = &[ArgumentGroupSpec {
    name: "source_category",
    required: true,
    exclusive: true,
    implicit_members: &["stdin"],
}];

#[rustfmt::skip]
const DETECT_ARGUMENTS: &[ArgumentSpec] = &[
    positional("TEXT", &[], false, false, Phase::TextDetection).in_group("source_category").with_stdin_marker("-").available(),
    option("--file", "PATH", &[], false, true, Phase::TextDetection).in_group("source_category").available(),
    flag("--detach", Phase::TextDetection).available(),
    option("--format", "FORMAT", OUTPUT_FORMATS, false, false, Phase::TextDetection).available(),
    flag("--include-input", Phase::TextDetection).available(),
    // Phase 4 Packet C: the contracted manual save path for completed
    // detection work. Bulk/task surfaces have no `--save`; they persist only
    // under the `history.enabled = true` automatic gate.
    flag("--save", Phase::History).available(),
    flag("--public-link", Phase::TextDetection).available(),
    option("--timeout", "DURATION", &[], false, false, Phase::TextDetection).available(),
    option("--progress", "MODE", PROGRESS_MODES, false, false, Phase::TextDetection).available(),
    option("--max-billable-units", "N", &[], false, false, Phase::TextDetection).available(),
];

#[rustfmt::skip]
const PLAGIARISM_ARGUMENTS: &[ArgumentSpec] = &[
    positional("TEXT", &[], false, false, Phase::FileAndPlagiarism).in_group("source_category").with_stdin_marker("-"),
    option("--file", "PATH", &[], false, true, Phase::FileAndPlagiarism).in_group("source_category"),
    option("--format", "FORMAT", OUTPUT_FORMATS, false, false, Phase::FileAndPlagiarism),
    flag("--include-input", Phase::FileAndPlagiarism),
    flag("--save", Phase::FileAndPlagiarism),
    option("--timeout", "DURATION", &[], false, false, Phase::FileAndPlagiarism),
    option("--progress", "MODE", PROGRESS_MODES, false, false, Phase::FileAndPlagiarism),
    option("--max-billable-units", "N", &[], false, false, Phase::FileAndPlagiarism),
];

#[rustfmt::skip]
const ANALYZE_ARGUMENTS: &[ArgumentSpec] = &[
    positional("TEXT", &[], false, false, Phase::FileAndPlagiarism).in_group("source_category").with_stdin_marker("-"),
    option("--file", "PATH", &[], false, true, Phase::FileAndPlagiarism).in_group("source_category"),
    option("--format", "FORMAT", OUTPUT_FORMATS, false, false, Phase::FileAndPlagiarism),
    flag("--include-input", Phase::FileAndPlagiarism),
    flag("--save", Phase::FileAndPlagiarism),
    flag("--public-link", Phase::FileAndPlagiarism),
    option("--timeout", "DURATION", &[], false, false, Phase::FileAndPlagiarism),
    option("--progress", "MODE", PROGRESS_MODES, false, false, Phase::FileAndPlagiarism),
    option("--max-billable-units", "N", &[], false, false, Phase::FileAndPlagiarism),
];

const BULK_SOURCE_GROUP: &[ArgumentGroupSpec] = &[ArgumentGroupSpec {
    name: "bulk_source",
    required: true,
    exclusive: true,
    implicit_members: &["stdin"],
}];

// Pangram's Bulk API documents no public-dashboard-link request or response
// field, so bulk submission has no `--public-link` option (contracts.md
// 14.3). The flag exists only on detect (available) and analyze (planned).
#[rustfmt::skip]
const BULK_SUBMIT_ARGUMENTS: &[ArgumentSpec] = &[
    positional("JSONL_PATH", &[], false, false, Phase::BulkAndTasks).in_group("bulk_source").with_stdin_marker("-").available(),
    option("--max-billable-units", "N", &[], true, false, Phase::BulkAndTasks).available(),
    flag("--dry-run", Phase::BulkAndTasks).available(),
    flag("--wait", Phase::BulkAndTasks).available(),
    option("--format", "FORMAT", OUTPUT_FORMATS, false, false, Phase::BulkAndTasks).available(),
    option("--progress", "MODE", PROGRESS_MODES, false, false, Phase::BulkAndTasks).available(),
];

#[rustfmt::skip]
const ID_ARGUMENTS: &[ArgumentSpec] = &[
    positional("ID", &[], true, false, Phase::BulkAndTasks).available(),
];

#[rustfmt::skip]
const WAIT_ARGUMENTS: &[ArgumentSpec] = &[
    positional("ID", &[], true, false, Phase::BulkAndTasks).available(),
    option("--timeout", "DURATION", &[], false, false, Phase::BulkAndTasks).available(),
    option("--progress", "MODE", PROGRESS_MODES, false, false, Phase::BulkAndTasks).available(),
];

#[rustfmt::skip]
const BULK_RESULTS_ARGUMENTS: &[ArgumentSpec] = &[
    positional("ID", &[], true, false, Phase::BulkAndTasks).available(),
    option("--offset", "N", &[], false, false, Phase::BulkAndTasks).available(),
    option("--limit", "N", &[], false, false, Phase::BulkAndTasks).available(),
    option("--format", "FORMAT", OUTPUT_FORMATS, false, false, Phase::BulkAndTasks).available(),
];

#[rustfmt::skip]
const HISTORY_LIST_ARGUMENTS: &[ArgumentSpec] = &[
    option("--status", "STATUS", &["queued", "running", "succeeded", "failed", "partial"], false, false, Phase::History).available(),
    option("--check", "CHECK", &["ai_detection", "plagiarism"], false, false, Phase::History).available(),
    option("--limit", "N", &[], false, false, Phase::History).available(),
];

#[rustfmt::skip]
const HISTORY_SHOW_ARGUMENTS: &[ArgumentSpec] = &[
    positional("ID", &[], true, false, Phase::History).available(),
    flag("--include-input", Phase::History).available(),
    option("--format", "FORMAT", OUTPUT_FORMATS, false, false, Phase::History).available(),
];

#[rustfmt::skip]
const HISTORY_SEARCH_ARGUMENTS: &[ArgumentSpec] = &[
    positional("QUERY", &[], true, false, Phase::History).available(),
    option("--status", "STATUS", &["queued", "running", "succeeded", "failed", "partial"], false, false, Phase::History).available(),
    option("--check", "CHECK", &["ai_detection", "plagiarism"], false, false, Phase::History).available(),
    option("--limit", "N", &[], false, false, Phase::History).available(),
];

#[rustfmt::skip]
const HISTORY_DELETE_ARGUMENTS: &[ArgumentSpec] = &[
    positional("ID", &[], true, false, Phase::History).available(),
    flag("--yes", Phase::History).available(),
];

#[rustfmt::skip]
const HISTORY_CLEAR_ARGUMENTS: &[ArgumentSpec] = &[
    flag("--yes", Phase::History).available(),
];

#[rustfmt::skip]
const HISTORY_EXPORT_ARGUMENTS: &[ArgumentSpec] = &[
    option("--format", "FORMAT", &["jsonl", "markdown"], false, false, Phase::History).available(),
    flag("--redact-content", Phase::History).available(),
];

#[rustfmt::skip]
const HISTORY_RERUN_ARGUMENTS: &[ArgumentSpec] = &[
    positional("ID", &[], true, false, Phase::History).available(),
    option("--format", "FORMAT", OUTPUT_FORMATS, false, false, Phase::History).available(),
    option("--progress", "MODE", PROGRESS_MODES, false, false, Phase::History).available(),
];

const API_KEY_GROUP: &[ArgumentGroupSpec] = &[ArgumentGroupSpec {
    name: "api_key_source",
    required: true,
    exclusive: true,
    implicit_members: &[],
}];

#[rustfmt::skip]
const AUTH_SET_ARGUMENTS: &[ArgumentSpec] = &[
    grouped_argument("--api-key", ArgumentKind::Option, Some("VALUE"), "api_key_source", Phase::LocalSetup).available(),
    grouped_argument("--api-key-stdin", ArgumentKind::Flag, None, "api_key_source", Phase::LocalSetup).available(),
];

#[rustfmt::skip]
const AUTH_LOGOUT_ARGUMENTS: &[ArgumentSpec] = &[
    flag("--yes", Phase::LocalSetup).available(),
];

#[rustfmt::skip]
const MCP_ARGUMENTS: &[ArgumentSpec] = &[
    flag("--history", Phase::McpAndAgents).available(),
    flag("--allow-history-mutations", Phase::McpAndAgents).requiring(&["--history"]).available(),
    flag("--allow-config-mutations", Phase::McpAndAgents).available(),
    flag("--allow-public-links", Phase::McpAndAgents).available(),
    option("--allow-file-root", "PATH", &[], false, true, Phase::McpAndAgents).available(),
];

const MCP_TARGET_GROUP: &[ArgumentGroupSpec] = &[ArgumentGroupSpec {
    name: "target_selection",
    required: true,
    exclusive: true,
    implicit_members: &[],
}];

#[rustfmt::skip]
const MCP_MUTATION_ARGUMENTS: &[ArgumentSpec] = &[
    option("--target", "CLIENT", MCP_CLIENTS, false, true, Phase::McpAndAgents).in_group("target_selection").available(),
    flag("--all", Phase::McpAndAgents).in_group("target_selection").available(),
    option("--server-name", "NAME", &[], false, false, Phase::McpAndAgents).available(),
    flag("--dry-run", Phase::McpAndAgents).available(),
];

#[rustfmt::skip]
const MCP_STATUS_ARGUMENTS: &[ArgumentSpec] = &[
    option("--format", "FORMAT", &["json", "pretty"], false, false, Phase::McpAndAgents).available(),
];

#[rustfmt::skip]
const SKILLS_GET_ARGUMENTS: &[ArgumentSpec] = &[
    positional("SKILL", &["pangram"], true, false, Phase::McpAndAgents).available(),
    flag("--full", Phase::McpAndAgents).available(),
];

#[rustfmt::skip]
const SKILLS_PATH_ARGUMENTS: &[ArgumentSpec] = &[
    positional("SKILL", &["pangram"], false, false, Phase::McpAndAgents).available(),
];

#[rustfmt::skip]
const CONFIG_GET_ARGUMENTS: &[ArgumentSpec] = &[
    positional("KEY", &[], true, false, Phase::LocalSetup).available(),
];

#[rustfmt::skip]
const CONFIG_SET_ARGUMENTS: &[ArgumentSpec] = &[
    positional("KEY", &[], true, false, Phase::LocalSetup).available(),
    positional("VALUE", &[], true, false, Phase::LocalSetup).available(),
];

#[rustfmt::skip]
const DOCTOR_ARGUMENTS: &[ArgumentSpec] = &[
    option("--format", "FORMAT", &["json", "pretty"], false, false, Phase::LocalSetup).available(),
];

#[rustfmt::skip]
const COMPLETIONS_ARGUMENTS: &[ArgumentSpec] = &[
    positional("SHELL", SHELLS, true, false, Phase::DistributionAndUpdate),
];

const UPDATE_MODE_GROUP: &[ArgumentGroupSpec] = &[ArgumentGroupSpec {
    name: "update_mode",
    required: false,
    exclusive: true,
    implicit_members: &[],
}];

#[rustfmt::skip]
const UPDATE_ARGUMENTS: &[ArgumentSpec] = &[
    grouped_argument("--check", ArgumentKind::Flag, None, "update_mode", Phase::DistributionAndUpdate),
    grouped_argument("--yes", ArgumentKind::Flag, None, "update_mode", Phase::DistributionAndUpdate),
];

/// The complete grammar remains generator data until each phase makes its
/// command executable. Runtime Clap definitions do not derive dormant command
/// entries from this table.
#[rustfmt::skip]
pub const FULL_GRAMMAR: GrammarSpec = GrammarSpec {
    name: "pangram",
    global_arguments: GLOBAL_ARGUMENTS,
    commands: &[
        CommandSpec {
            path: &[], kind: CommandKind::Entrypoint, arguments: ROOT_ARGUMENTS,
            argument_groups: SOURCE_CATEGORY_GROUP, phase: Phase::Scaffold, availability: Availability::Available,
        },
        available_command(&["detect"], CommandKind::Command, DETECT_ARGUMENTS, SOURCE_CATEGORY_GROUP, Phase::TextDetection),
        planned_command(&["plagiarism"], CommandKind::Command, PLAGIARISM_ARGUMENTS, SOURCE_CATEGORY_GROUP, Phase::FileAndPlagiarism),
        planned_command(&["analyze"], CommandKind::Command, ANALYZE_ARGUMENTS, SOURCE_CATEGORY_GROUP, Phase::FileAndPlagiarism),
        available_command(&["bulk"], CommandKind::Namespace, &[], &[], Phase::BulkAndTasks),
        available_command(&["bulk", "submit"], CommandKind::Command, BULK_SUBMIT_ARGUMENTS, BULK_SOURCE_GROUP, Phase::BulkAndTasks),
        available_command(&["bulk", "status"], CommandKind::Command, ID_ARGUMENTS, &[], Phase::BulkAndTasks),
        available_command(&["bulk", "wait"], CommandKind::Command, WAIT_ARGUMENTS, &[], Phase::BulkAndTasks),
        available_command(&["bulk", "results"], CommandKind::Command, BULK_RESULTS_ARGUMENTS, &[], Phase::BulkAndTasks),
        available_command(&["task"], CommandKind::Namespace, &[], &[], Phase::BulkAndTasks),
        available_command(&["task", "status"], CommandKind::Command, ID_ARGUMENTS, &[], Phase::BulkAndTasks),
        available_command(&["task", "wait"], CommandKind::Command, WAIT_ARGUMENTS, &[], Phase::BulkAndTasks),
        available_command(&["history"], CommandKind::Namespace, &[], &[], Phase::History),
        available_command(&["history", "list"], CommandKind::Command, HISTORY_LIST_ARGUMENTS, &[], Phase::History),
        available_command(&["history", "show"], CommandKind::Command, HISTORY_SHOW_ARGUMENTS, &[], Phase::History),
        available_command(&["history", "search"], CommandKind::Command, HISTORY_SEARCH_ARGUMENTS, &[], Phase::History),
        available_command(&["history", "delete"], CommandKind::Command, HISTORY_DELETE_ARGUMENTS, &[], Phase::History),
        available_command(&["history", "clear"], CommandKind::Command, HISTORY_CLEAR_ARGUMENTS, &[], Phase::History),
        available_command(&["history", "export"], CommandKind::Command, HISTORY_EXPORT_ARGUMENTS, &[], Phase::History),
        available_command(&["history", "rerun"], CommandKind::Command, HISTORY_RERUN_ARGUMENTS, &[], Phase::History),
        available_command(&["auth"], CommandKind::Command, &[], &[], Phase::LocalSetup),
        available_command(&["auth", "set"], CommandKind::Command, AUTH_SET_ARGUMENTS, API_KEY_GROUP, Phase::LocalSetup),
        available_command(&["auth", "status"], CommandKind::Command, &[], &[], Phase::LocalSetup),
        available_command(&["auth", "logout"], CommandKind::Command, AUTH_LOGOUT_ARGUMENTS, &[], Phase::LocalSetup),
        available_command(&["mcp"], CommandKind::Command, MCP_ARGUMENTS, &[], Phase::McpAndAgents),
        available_command(&["mcp", "install"], CommandKind::Command, MCP_MUTATION_ARGUMENTS, MCP_TARGET_GROUP, Phase::McpAndAgents),
        available_command(&["mcp", "uninstall"], CommandKind::Command, MCP_MUTATION_ARGUMENTS, MCP_TARGET_GROUP, Phase::McpAndAgents),
        available_command(&["mcp", "status"], CommandKind::Command, MCP_STATUS_ARGUMENTS, &[], Phase::McpAndAgents),
        available_command(&["agent"], CommandKind::Command, &[], &[], Phase::McpAndAgents),
        available_command(&["skills"], CommandKind::Namespace, &[], &[], Phase::McpAndAgents),
        available_command(&["skills", "list"], CommandKind::Command, &[], &[], Phase::McpAndAgents),
        available_command(&["skills", "get"], CommandKind::Command, SKILLS_GET_ARGUMENTS, &[], Phase::McpAndAgents),
        available_command(&["skills", "path"], CommandKind::Command, SKILLS_PATH_ARGUMENTS, &[], Phase::McpAndAgents),
        available_command(&["config"], CommandKind::Namespace, &[], &[], Phase::LocalSetup),
        available_command(&["config", "list"], CommandKind::Command, &[], &[], Phase::LocalSetup),
        available_command(&["config", "get"], CommandKind::Command, CONFIG_GET_ARGUMENTS, &[], Phase::LocalSetup),
        available_command(&["config", "set"], CommandKind::Command, CONFIG_SET_ARGUMENTS, &[], Phase::LocalSetup),
        available_command(&["config", "path"], CommandKind::Command, &[], &[], Phase::LocalSetup),
        available_command(&["doctor"], CommandKind::Command, DOCTOR_ARGUMENTS, &[], Phase::LocalSetup),
        planned_command(&["completions"], CommandKind::Command, COMPLETIONS_ARGUMENTS, &[], Phase::DistributionAndUpdate),
        planned_command(&["update"], CommandKind::Command, UPDATE_ARGUMENTS, UPDATE_MODE_GROUP, Phase::DistributionAndUpdate),
    ],
};

/// Returns the ordered full grammar without exposing planned commands to Clap.
pub const fn full_grammar_reference() -> &'static GrammarSpec {
    &FULL_GRAMMAR
}
