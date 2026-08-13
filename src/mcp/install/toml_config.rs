//! Surgical editor for Codex's global `[mcp_servers.<name>]` table.

use toml::Value;

use super::jsonc::{EditError, EditOutcome};
use super::{InstallAction, InstallChange};

pub(super) fn edit_codex_toml(
    source: &str,
    server_name: &str,
    executable: &str,
    action: InstallAction,
) -> Result<EditOutcome, EditError> {
    let document = if source.is_empty() {
        Value::Table(Default::default())
    } else {
        toml::from_str::<Value>(source)
            .map_err(|_| EditError::Malformed("invalid TOML syntax".into()))?
    };
    let existing = document
        .as_table()
        .and_then(|root| root.get("mcp_servers"))
        .and_then(Value::as_table)
        .and_then(|servers| servers.get(server_name));
    let owned = existing.is_some_and(|entry| is_owned_entry(entry, executable));

    match (action, existing, owned) {
        (InstallAction::Install, None, _) => Ok(EditOutcome {
            change: InstallChange::Create,
            replacement: Some(append_table(source, server_name, executable)),
        }),
        (InstallAction::Uninstall, None, _) => Ok(EditOutcome {
            change: InstallChange::Unchanged,
            replacement: None,
        }),
        (InstallAction::Install, Some(_), true) => Ok(EditOutcome {
            change: InstallChange::Unchanged,
            replacement: None,
        }),
        (InstallAction::Uninstall, Some(_), true) => {
            let (start, end) = find_table_span(source, server_name).ok_or(EditError::Conflict)?;
            let mut replacement = String::with_capacity(source.len() - (end - start));
            replacement.push_str(&source[..start]);
            replacement.push_str(&source[end..]);
            Ok(EditOutcome {
                change: InstallChange::Remove,
                replacement: Some(replacement),
            })
        }
        (_, Some(_), false) => Err(EditError::Conflict),
    }
}

fn is_owned_entry(entry: &Value, executable: &str) -> bool {
    let Some(table) = entry.as_table() else {
        return false;
    };
    table.len() == 2
        && table.get("command").and_then(Value::as_str) == Some(executable)
        && table
            .get("args")
            .and_then(Value::as_array)
            .is_some_and(|args| args.len() == 1 && args[0].as_str() == Some("mcp"))
}

fn append_table(source: &str, server_name: &str, executable: &str) -> String {
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let separator = if source.is_empty() || source.ends_with("\r\n\r\n") || source.ends_with("\n\n")
    {
        ""
    } else if source.ends_with('\n') {
        newline
    } else if newline == "\r\n" {
        "\r\n\r\n"
    } else {
        "\n\n"
    };
    format!(
        "{source}{separator}{}{newline}command = {}{newline}args = [\"mcp\"]{newline}",
        table_header(server_name),
        basic_string(executable),
    )
}

fn table_header(server_name: &str) -> String {
    if server_name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        format!("[mcp_servers.{server_name}]")
    } else {
        format!("[mcp_servers.{}]", basic_string(server_name))
    }
}

fn basic_string(value: &str) -> String {
    // JSON and TOML basic strings share the escapes needed by paths and
    // validated server names. serde_json never emits the optional `\/` form.
    serde_json::to_string(value).expect("strings are serializable")
}

fn find_table_span(source: &str, server_name: &str) -> Option<(usize, usize)> {
    let lines = line_spans(source);
    let header_index = lines
        .iter()
        .position(|(start, end)| is_server_table_header(&source[*start..*end], server_name))?;
    let header_start = lines[header_index].0;
    let mut start = header_start;
    if header_index > 0 {
        let (prior_start, prior_end) = lines[header_index - 1];
        if source[prior_start..prior_end].trim().is_empty() {
            start = prior_start;
        }
    }
    let end = lines
        .iter()
        .skip(header_index + 1)
        .find_map(|(start, end)| {
            let trimmed = source[*start..*end].trim_start();
            trimmed.starts_with('[').then_some(*start)
        })
        .unwrap_or(source.len());
    Some((start, end))
}

fn is_server_table_header(line: &str, server_name: &str) -> bool {
    let Ok(Value::Table(root)) = toml::from_str::<Value>(line) else {
        return false;
    };
    root.len() == 1
        && root
            .get("mcp_servers")
            .and_then(Value::as_table)
            .is_some_and(|servers| {
                servers.len() == 1
                    && servers
                        .get(server_name)
                        .and_then(Value::as_table)
                        .is_some_and(|entry| entry.is_empty())
            })
}

fn line_spans(source: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0;
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            spans.push((start, index + 1));
            start = index + 1;
        }
    }
    if start < source.len() {
        spans.push((start, source.len()));
    }
    spans
}
