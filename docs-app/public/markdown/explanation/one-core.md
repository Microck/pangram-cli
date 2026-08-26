# Why one core serves CLI, TUI, and MCP

CLI, TUI, and MCP are adapters. They call the same Rust analysis module, which
owns submission, polling, retries, normalized status, and upstream contract
validation. Output projection starts from the same typed result. This keeps a
protocol fix from becoming three separate behavior changes.
