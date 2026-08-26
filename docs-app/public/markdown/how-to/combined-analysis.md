# Run a combined analysis

```bash
pangram analyze 'Synthetic text for both checks.' --max-billable-units 6
```

Both billable checks start together. The result preserves canonical check
order and keeps either successful half if the other check fails.
