# Use Pangram from Nushell

## Install completions

`pangram completions nushell` prints a Nushell module of `extern` signatures.
Save it into a vendor autoload directory so every new Nushell session loads it:

```nu
mkdir ($nu.data-dir | path join vendor autoload)
pangram completions nushell | save -f ($nu.data-dir | path join vendor autoload pangram.nu)
```

Start a new session, then type `pangram ` and press Tab. Run the same command
again after each `pangram update` so completions match the new version.

## Read one result as a table

Each analysis command prints one canonical JSON envelope on stdout. `from json`
turns it into a Nushell record:

```nu
let report = pangram detect --file essay.md --format json | from json
$report.data.status
$report.data.checks | select kind status
```

Segments come out as a table:

```nu
$report.data.checks.0.result.segments | select label confidence ai_assistance_score word_count
```

## Read several files

`--format jsonl` prints one envelope per file, one per line. Parse them with
`from json --objects`:

```nu
pangram detect --file a.md --file b.md --format jsonl
| from json --objects
| each {|row| {file: $row.data.input.name, classification: $row.data.checks.0.result.classification}}
```

## Handle failures

A failed run still prints a JSON envelope on stdout, with `error` in place of
`data`, and exits nonzero. Nushell treats the nonzero exit as an error, so
`complete` keeps the output available:

```nu
let run = pangram detect --file essay.md --format json | complete
let report = $run.stdout | from json
if $run.exit_code != 0 { $report.error | select code message }
```

The envelope fields are described in the
[output schema reference](/docs/reference/output-schema).
