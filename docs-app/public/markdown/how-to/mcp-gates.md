# Gate MCP capabilities and file access

```bash
pangram mcp --history --allow-file-root /absolute/safe/root
```

History reads, history mutations, configuration mutations, public links, and
file roots use separate startup gates. Approved roots are opened before the
server reads stdin and reject symlink or reparse-point traversal.
