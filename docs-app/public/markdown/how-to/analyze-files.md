# Analyze PDF, DOCX, and RTF files

```bash
pangram detect --file sample.pdf --file sample.docx --max-billable-units 2
```

UTF-8 text, PDF, DOCX, and RTF are supported for AI detection. File requests
are billable. MCP does not expose binary files because it cannot estimate the
server-extracted word count before submission.
