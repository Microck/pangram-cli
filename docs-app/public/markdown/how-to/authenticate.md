# Configure an API key

```bash
printf '%s\n' "$PANGRAM_API_KEY" | pangram auth set --api-key-stdin
pangram auth status
```

`PANGRAM_API_KEY` overrides stored credentials. The CLI never prints the full
key. Authentication setup is not billable.
