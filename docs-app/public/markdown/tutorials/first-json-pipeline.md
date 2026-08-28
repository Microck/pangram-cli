# Your first JSON pipeline

```bash
printf '%s\n' 'A synthetic sample for automation.' | pangram - > result.json
jq '.command, .data.status' result.json
```

The analysis request is billable. The canonical envelope goes to stdout.
Progress and interactive text never enter stdout. A nonzero exit code means
the command did not produce a complete success.
