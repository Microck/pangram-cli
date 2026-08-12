# Pangram MCP agent reference

Protocol: MCP 2026-07-28 over stdio.

## Tools

- `detect_text`: Detect AI-written text with Pangram 4.
- `get_task`: Get one Pangram task without waiting.
- `wait_task`: Wait for one Pangram task to reach a terminal state.
- `submit_bulk`: Submit inline items or one approved JSONL file to Pangram 4.
- `get_bulk`: Get one Pangram bulk job without waiting.
- `wait_bulk`: Wait for one Pangram bulk job to reach a terminal state.
- `get_bulk_results`: Get one explicit results page for a Pangram bulk job.
- `history_list`: List saved local analysis summaries.
- `history_search`: Search saved local analysis summaries with literal text.
- `history_get`: Get one saved local analysis, redacted by default.
- `history_rerun`: Submit the saved input from one local analysis again.
- `history_delete`: Delete one saved local analysis.
- `history_clear`: Delete all saved local analyses.
- `update_config`: Set one supported non-secret Pangram CLI configuration key.

## Safety

Every billable submission requires `max_billable_units`. Local IDs require history; upstream IDs do not. History reads, history mutations, configuration mutations, public links, and file roots require their explicit startup capabilities. `save: true` requires history and history mutations. File paths must be inside an approved root. Cancellation stops local observation only.
