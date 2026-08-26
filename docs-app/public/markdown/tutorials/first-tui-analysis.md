# Your first TUI analysis

## 1. Store the API key

```bash
printf '%s\n' "$PANGRAM_API_KEY" | pangram auth set --api-key-stdin
```

## 2. Open the TUI

```bash
pangram
```

Choose whether the TUI may check for CLI updates. Enter synthetic text, then
submit it. The request is billable. The TUI shows the overall label and the
ordered segment evidence returned by Pangram. Local history stays off unless
you enable it.
