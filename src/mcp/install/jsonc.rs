//! Minimal source-preserving JSON/JSONC editor for MCP registration maps.
//!
//! This is intentionally not a general formatter. It parses the complete
//! document to reject malformed input and duplicate keys, then changes only
//! the selected object member span. Comments and trailing commas are enabled
//! only for clients whose current format documents JSONC-style input.

use serde_json::{Map, Number, Value};

use super::{InstallAction, InstallChange};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JsonFormat {
    Strict,
    Jsonc,
}

#[derive(Debug)]
pub(super) struct EditOutcome {
    pub(super) change: InstallChange,
    pub(super) replacement: Option<String>,
}

#[derive(Debug)]
pub(super) enum EditError {
    Malformed(String),
    DuplicateKey(String),
    Conflict,
}

pub(super) fn edit_json(
    source: &str,
    container_name: &str,
    server_name: &str,
    expected_entry: &Value,
    action: InstallAction,
    format: JsonFormat,
) -> Result<EditOutcome, EditError> {
    let was_missing = source.is_empty();
    let source = if was_missing { "{}\n" } else { source };
    let mut parser = Parser::new(source, format);
    let root = parser.parse_document()?;
    let root_object = root
        .object
        .as_ref()
        .ok_or_else(|| EditError::Malformed("configuration root must be an object".into()))?;

    let Some(container_field) = root_object.field(container_name) else {
        return match action {
            InstallAction::Uninstall => Ok(EditOutcome {
                change: InstallChange::Unchanged,
                replacement: None,
            }),
            InstallAction::Install => {
                let nested = Value::Object(Map::from_iter([(
                    server_name.to_owned(),
                    expected_entry.clone(),
                )]));
                Ok(EditOutcome {
                    change: InstallChange::Create,
                    replacement: Some(insert_member(source, root_object, container_name, &nested)?),
                })
            }
        };
    };
    let container = container_field
        .node
        .object
        .as_ref()
        .ok_or_else(|| EditError::Malformed(format!("{container_name} must be an object")))?;
    let existing = container
        .fields
        .iter()
        .enumerate()
        .find(|(_, field)| field.key == server_name);
    match (action, existing) {
        (InstallAction::Install, None) => Ok(EditOutcome {
            change: InstallChange::Create,
            replacement: Some(insert_member(
                source,
                container,
                server_name,
                expected_entry,
            )?),
        }),
        (InstallAction::Uninstall, None) => Ok(EditOutcome {
            change: InstallChange::Unchanged,
            replacement: None,
        }),
        (InstallAction::Install, Some((_, field))) if field.node.value == *expected_entry => {
            Ok(EditOutcome {
                change: InstallChange::Unchanged,
                replacement: None,
            })
        }
        (InstallAction::Uninstall, Some((index, field))) if field.node.value == *expected_entry => {
            Ok(EditOutcome {
                change: InstallChange::Remove,
                replacement: Some(remove_member(source, container, index)),
            })
        }
        (_, Some(_)) => Err(EditError::Conflict),
    }
}

#[derive(Debug)]
struct Node {
    value: Value,
    end: usize,
    object: Option<ObjectNode>,
}

#[derive(Debug)]
struct ObjectNode {
    open: usize,
    close: usize,
    fields: Vec<FieldNode>,
}

impl ObjectNode {
    fn field(&self, key: &str) -> Option<&FieldNode> {
        self.fields.iter().find(|field| field.key == key)
    }
}

#[derive(Debug)]
struct FieldNode {
    key: String,
    key_start: usize,
    node: Node,
    comma: Option<usize>,
}

struct Parser<'a> {
    source: &'a str,
    bytes: &'a [u8],
    cursor: usize,
    format: JsonFormat,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str, format: JsonFormat) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            cursor: 0,
            format,
        }
    }

    fn parse_document(&mut self) -> Result<Node, EditError> {
        self.skip_trivia()?;
        let node = self.parse_value()?;
        self.skip_trivia()?;
        if self.cursor != self.bytes.len() {
            return self.malformed("unexpected bytes after the root value");
        }
        Ok(node)
    }

    fn parse_value(&mut self) -> Result<Node, EditError> {
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => {
                let value = Value::String(self.parse_string()?);
                Ok(Node {
                    value,
                    end: self.cursor,
                    object: None,
                })
            }
            Some(b't') => self.parse_literal(b"true", Value::Bool(true)),
            Some(b'f') => self.parse_literal(b"false", Value::Bool(false)),
            Some(b'n') => self.parse_literal(b"null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            _ => self.malformed("expected a JSON value"),
        }
    }

    fn parse_object(&mut self) -> Result<Node, EditError> {
        let open = self.cursor;
        self.cursor += 1;
        let mut fields = Vec::new();
        let mut values = Map::new();
        self.skip_trivia()?;
        if self.consume(b'}') {
            return Ok(Node {
                value: Value::Object(values),
                end: self.cursor,
                object: Some(ObjectNode {
                    open,
                    close: self.cursor - 1,
                    fields,
                }),
            });
        }
        loop {
            self.skip_trivia()?;
            let key_start = self.cursor;
            if self.peek() != Some(b'"') {
                return self.malformed("object keys must be quoted strings");
            }
            let key = self.parse_string()?;
            if values.contains_key(&key) {
                return Err(EditError::DuplicateKey(key));
            }
            self.skip_trivia()?;
            self.expect(b':', "expected ':' after object key")?;
            self.skip_trivia()?;
            let node = self.parse_value()?;
            values.insert(key.clone(), node.value.clone());
            self.skip_trivia()?;
            let comma = if self.consume(b',') {
                let comma = self.cursor - 1;
                self.skip_trivia()?;
                if self.peek() == Some(b'}') && self.format == JsonFormat::Strict {
                    return self.malformed("trailing commas are not allowed in strict JSON");
                }
                Some(comma)
            } else {
                None
            };
            fields.push(FieldNode {
                key,
                key_start,
                node,
                comma,
            });
            if self.consume(b'}') {
                let close = self.cursor - 1;
                return Ok(Node {
                    value: Value::Object(values),
                    end: self.cursor,
                    object: Some(ObjectNode {
                        open,
                        close,
                        fields,
                    }),
                });
            }
            if comma.is_none() {
                return self.malformed("expected ',' or '}' after object value");
            }
        }
    }

    fn parse_array(&mut self) -> Result<Node, EditError> {
        self.cursor += 1;
        let mut values = Vec::new();
        self.skip_trivia()?;
        if self.consume(b']') {
            return Ok(Node {
                value: Value::Array(values),
                end: self.cursor,
                object: None,
            });
        }
        loop {
            self.skip_trivia()?;
            values.push(self.parse_value()?.value);
            self.skip_trivia()?;
            if self.consume(b']') {
                break;
            }
            self.expect(b',', "expected ',' or ']' after array value")?;
            self.skip_trivia()?;
            if self.peek() == Some(b']') {
                if self.format == JsonFormat::Strict {
                    return self.malformed("trailing commas are not allowed in strict JSON");
                }
                self.cursor += 1;
                break;
            }
        }
        Ok(Node {
            value: Value::Array(values),
            end: self.cursor,
            object: None,
        })
    }

    fn parse_string(&mut self) -> Result<String, EditError> {
        let start = self.cursor;
        self.cursor += 1;
        let mut escaped = false;
        while let Some(byte) = self.peek() {
            self.cursor += 1;
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'"' => {
                    return serde_json::from_str(&self.source[start..self.cursor])
                        .map_err(|_| EditError::Malformed("invalid string escape".into()));
                }
                0x00..=0x1f => return self.malformed("control character in string"),
                _ => {}
            }
        }
        self.malformed("unterminated string")
    }

    fn parse_literal(&mut self, literal: &[u8], value: Value) -> Result<Node, EditError> {
        if !self.bytes[self.cursor..].starts_with(literal) {
            return self.malformed("invalid JSON literal");
        }
        self.cursor += literal.len();
        Ok(Node {
            value,
            end: self.cursor,
            object: None,
        })
    }

    fn parse_number(&mut self) -> Result<Node, EditError> {
        let start = self.cursor;
        while matches!(
            self.peek(),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.cursor += 1;
        }
        let number = serde_json::from_str::<Number>(&self.source[start..self.cursor])
            .map_err(|_| EditError::Malformed("invalid JSON number".into()))?;
        Ok(Node {
            value: Value::Number(number),
            end: self.cursor,
            object: None,
        })
    }

    fn skip_trivia(&mut self) -> Result<(), EditError> {
        loop {
            while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
                self.cursor += 1;
            }
            if self.bytes.get(self.cursor..self.cursor + 2) == Some(b"//") {
                if self.format == JsonFormat::Strict {
                    return self.malformed("comments are not allowed in strict JSON");
                }
                self.cursor += 2;
                while !matches!(self.peek(), None | Some(b'\n' | b'\r')) {
                    self.cursor += 1;
                }
            } else if self.bytes.get(self.cursor..self.cursor + 2) == Some(b"/*") {
                if self.format == JsonFormat::Strict {
                    return self.malformed("comments are not allowed in strict JSON");
                }
                self.cursor += 2;
                while self.bytes.get(self.cursor..self.cursor + 2) != Some(b"*/") {
                    if self.peek().is_none() {
                        return self.malformed("unterminated block comment");
                    }
                    self.cursor += 1;
                }
                self.cursor += 2;
            } else {
                return Ok(());
            }
        }
    }

    fn expect(&mut self, byte: u8, message: &'static str) -> Result<(), EditError> {
        if self.consume(byte) {
            Ok(())
        } else {
            self.malformed(message)
        }
    }

    fn consume(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }

    fn malformed<T>(&self, message: impl Into<String>) -> Result<T, EditError> {
        Err(EditError::Malformed(format!(
            "{} at byte {}",
            message.into(),
            self.cursor
        )))
    }
}

fn insert_member(
    source: &str,
    object: &ObjectNode,
    key: &str,
    value: &Value,
) -> Result<String, EditError> {
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let base_indent = line_indent(source, object.open);
    let member_indent = object
        .fields
        .first()
        .map(|field| line_indent(source, field.key_start))
        .filter(|indent| indent.len() > base_indent.len())
        .unwrap_or_else(|| format!("{base_indent}  "));
    let encoded_key = serde_json::to_string(key)
        .map_err(|error| EditError::Malformed(format!("cannot encode key: {error}")))?;
    let encoded_value = serde_json::to_string(value)
        .map_err(|error| EditError::Malformed(format!("cannot encode entry: {error}")))?;
    let member = format!("{encoded_key}: {encoded_value}");

    let (at, insertion) = if let Some(last) = object.fields.last() {
        let at = last.comma.map_or(last.node.end, |comma| comma + 1);
        let (leading_comma, trailing_comma) = if last.comma.is_some() {
            // Preserve an existing trailing comma as the prior member's
            // separator and give the inserted last member its own trailing
            // comma. Uninstall can then remove only the inserted span and
            // restore the original bytes exactly.
            ("", ",")
        } else {
            (",", "")
        };
        (
            at,
            format!("{leading_comma}{newline}{member_indent}{member}{trailing_comma}"),
        )
    } else {
        (
            object.close,
            format!("{newline}{member_indent}{member}{newline}{base_indent}"),
        )
    };
    let mut edited = String::with_capacity(source.len() + insertion.len());
    edited.push_str(&source[..at]);
    edited.push_str(&insertion);
    edited.push_str(&source[at..]);
    Ok(edited)
}

fn remove_member(source: &str, object: &ObjectNode, index: usize) -> String {
    let selected = &object.fields[index];
    let (start, end) = if object.fields.len() == 1 {
        (
            selected.key_start,
            selected.comma.map_or(selected.node.end, |comma| comma + 1),
        )
    } else if index > 0 {
        let prior = &object.fields[index - 1];
        let (start, end) = if let Some(selected_comma) = selected.comma {
            // The prior comma existed before an insertion into a
            // trailing-comma object. Keep it and remove the inserted member's
            // trailing comma.
            (
                prior.comma.map_or(prior.node.end, |comma| comma + 1),
                selected_comma + 1,
            )
        } else {
            // The prior comma was inserted as the separator. Remove it with
            // the selected final member to restore the original bytes.
            (prior.comma.unwrap_or(prior.node.end), selected.node.end)
        };
        (start, end)
    } else {
        (selected.key_start, object.fields[1].key_start)
    };
    let mut edited = String::with_capacity(source.len() - (end - start));
    edited.push_str(&source[..start]);
    edited.push_str(&source[end..]);
    edited
}

fn line_indent(source: &str, offset: usize) -> String {
    let line_start = source[..offset]
        .rfind('\n')
        .map_or(0, |newline| newline + 1);
    source[line_start..offset]
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t'))
        .collect()
}
