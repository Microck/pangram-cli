# Connect your first MCP client

Preview the exact client edit first:

```bash
pangram mcp install --target cursor --dry-run
pangram mcp install --target cursor
```

Restart the client, then call `detect_text` with synthetic `text` and a
positive `max_billable_units`. The tool is billable and cannot submit above
that ceiling. Default MCP startup exposes no history mutations, public links,
or file roots.
