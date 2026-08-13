use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use serde_json::{Value, json};

pub struct McpProcess {
    _root: tempfile::TempDir,
    child: Child,
    stdin: Option<ChildStdin>,
    responses: Receiver<String>,
    stderr: Receiver<String>,
    next_id: u64,
}

impl McpProcess {
    pub fn spawn(arguments: &[&str]) -> Self {
        Self::spawn_with_env(arguments, &[])
    }

    pub fn spawn_with_env(arguments: &[&str], environment: &[(&str, &OsStr)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_pangram"));
        command
            .arg("mcp")
            .args(arguments)
            .env_remove("PANGRAM_API_KEY");
        Self::spawn_command(command, environment)
    }

    #[cfg(feature = "dev-tools")]
    pub fn spawn_loopback(base_url: &str, arguments: &[&str]) -> Self {
        Self::spawn_loopback_with_env(base_url, arguments, &[])
    }

    #[cfg(feature = "dev-tools")]
    pub fn spawn_loopback_with_env(
        base_url: &str,
        arguments: &[&str],
        environment: &[(&str, &OsStr)],
    ) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_pangram-test-driver"));
        command.arg(base_url).arg("mcp").args(arguments).env(
            "PANGRAM_API_KEY",
            "pg_fixture_synthetic_key_00000000000000000000",
        );
        Self::spawn_command(command, environment)
    }

    fn spawn_command(mut command: Command, environment: &[(&str, &OsStr)]) -> Self {
        let root = tempfile::tempdir().expect("private MCP test root");
        let home = root.path().join("home");
        let config_home = root.path().join("config-home");
        let data_home = root.path().join("data-home");
        let data_dir = root.path().join("pangram-data");
        for directory in [&home, &config_home, &data_home, &data_dir] {
            fs::create_dir_all(directory).expect("create MCP test directory");
        }
        command
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", &config_home)
            .env("XDG_DATA_HOME", &data_home)
            .env("PANGRAM_CONFIG", root.path().join("config.toml"))
            .env("PANGRAM_DATA_DIR", &data_dir)
            .env("CI", "true")
            .env("TERM", "dumb")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, value) in environment {
            command.env(name, value);
        }
        let mut child = command.spawn().expect("spawn pangram mcp");
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let (sender, responses) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
        });
        let (stderr_sender, stderr_receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut contents = String::new();
            let _ = stderr.read_to_string(&mut contents);
            let _ = stderr_sender.send(contents);
        });

        Self {
            _root: root,
            child,
            stdin: Some(stdin),
            responses,
            stderr: stderr_receiver,
            next_id: 1,
        }
    }

    pub fn discover(&mut self) -> Value {
        self.request("server/discover", json!({}), true)
    }

    pub fn request(&mut self, method: &str, params: Value, with_meta: bool) -> Value {
        let id = self.start_request(method, params, with_meta);
        let response = self
            .response_within(Duration::from_secs(10))
            .expect("MCP response before timeout");
        assert_eq!(response["id"], id);
        response
    }

    /// Starts a request without waiting for its response. Cancellation tests
    /// use this to keep one observation in flight while sending the matching
    /// notification over the same real stdio session.
    pub fn start_request(&mut self, method: &str, mut params: Value, with_meta: bool) -> u64 {
        if with_meta {
            params.as_object_mut().expect("object params").insert(
                "_meta".to_owned(),
                json!({
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientCapabilities": {}
                }),
            );
        }
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        id
    }

    /// Returns `None` only when no response arrives within `timeout`. A closed
    /// stdout channel is a server failure, not evidence that a request was
    /// correctly suppressed.
    pub fn response_within(&self, timeout: Duration) -> Option<Value> {
        match self.responses.recv_timeout(timeout) {
            Ok(line) => Some(serde_json::from_str(&line).expect("JSON-RPC response")),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => panic!("MCP stdout closed unexpectedly"),
        }
    }

    pub fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().expect("open MCP stdin");
        serde_json::to_writer(&mut *stdin, &message).expect("write request");
        stdin.write_all(b"\n").expect("terminate request");
        stdin.flush().expect("flush request");
    }

    pub fn shutdown(mut self) -> String {
        drop(self.stdin.take());
        let status = self.child.wait().expect("wait for MCP server");
        assert!(status.success(), "MCP server exited with {status}");
        self.stderr
            .recv_timeout(Duration::from_secs(10))
            .expect("collect MCP stderr")
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        if self.stdin.is_some() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn result(response: &Value) -> &Value {
    response
        .get("result")
        .unwrap_or_else(|| panic!("expected success response, got {response}"))
}
