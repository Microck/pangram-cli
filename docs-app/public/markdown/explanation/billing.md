# Billing and billable-unit estimates

Pangram 4 text estimation uses one unit per started 100-word block, with a
minimum of one. Text plagiarism uses a fixed five-unit estimate. Combined text
analysis sums both. Bulk sums item estimates and enforces Pangram's 1,000-unit
request limit. Estimates are preflight controls, not account balance checks.
