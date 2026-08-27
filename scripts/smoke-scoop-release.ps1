param(
  [Parameter(Mandatory = $true)][string]$Artifacts,
  [Parameter(Mandatory = $true)][string]$ScoopSource,
  [Parameter(Mandatory = $true)][string]$Version
)

$ErrorActionPreference = "Stop"
$artifacts = (Resolve-Path $Artifacts).Path
$scoopSource = (Resolve-Path $ScoopSource).Path
$root = Join-Path $env:RUNNER_TEMP ("pangram-scoop-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $root | Out-Null

$server = $null
$current = $null
try {
  $manifest = Join-Path $root "pangram.json"
  $archiveName = "pangram-v$Version-x86_64-pc-windows-msvc.zip"
  $productionUrl = "https://github.com/Microck/pangram-cli/releases/download/v$Version/$archiveName"
  $serverOrigin = "http://127.0.0.1:18765"
  $contents = Get-Content -LiteralPath (Join-Path $artifacts "pangram-scoop.json") -Raw
  if ([regex]::Matches($contents, [regex]::Escape($productionUrl)).Count -ne 1) {
    throw "generated Scoop manifest has an unexpected release URL set"
  }
  $contents.Replace($productionUrl, "$serverOrigin/$archiveName") |
    Set-Content -LiteralPath $manifest -Encoding utf8

  $server = Start-Process -FilePath (Get-Command python).Source -ArgumentList @(
    "-m", "http.server", "18765", "--bind", "127.0.0.1", "--directory", $artifacts
  ) -RedirectStandardOutput (Join-Path $root "http.stdout.log") `
    -RedirectStandardError (Join-Path $root "http.stderr.log") -PassThru

  $ready = $false
  for ($attempt = 0; $attempt -lt 50; $attempt++) {
    try {
      Microsoft.PowerShell.Utility\Invoke-WebRequest -UseBasicParsing `
        -Method Head -Uri "$serverOrigin/$archiveName" | Out-Null
      $ready = $true
      break
    } catch {
      Start-Sleep -Milliseconds 100
    }
  }
  if (-not $ready) { throw "local release server did not start" }

  $env:SCOOP = Join-Path $root "scoop"
  $env:SCOOP_GLOBAL = Join-Path $root "global"
  $current = Join-Path $env:SCOOP "apps\pangram\current"
  New-Item -ItemType Directory -Path (Join-Path $env:SCOOP "buckets") | Out-Null
  New-Item -ItemType Directory -Path (Join-Path $env:SCOOP "shims") | Out-Null
  $scoop = Join-Path $scoopSource "bin/scoop.ps1"
  & $scoop config aria2-enabled false | Out-Null
  & $scoop install --no-update-scoop $manifest
  if ($LASTEXITCODE -ne 0) { throw "Scoop refused the generated manifest" }

  $installed = & (Join-Path $env:SCOOP "shims/pangram.exe") --version
  if ($installed -ne "pangram $Version") {
    throw "Scoop-installed binary version mismatch"
  }
} finally {
  if ($null -ne $server -and -not $server.HasExited) {
    Stop-Process -Id $server.Id -Force
    $server.WaitForExit()
  }
  # Scoop marks its current-version junction read-only. Remove that junction
  # directly before deleting the isolated smoke root.
  if ($null -ne $current -and (Test-Path -LiteralPath $current)) {
    & attrib.exe -R $current /L
    if ($LASTEXITCODE -ne 0) { throw "failed to make the Scoop junction removable" }
    [IO.Directory]::Delete($current)
  }
  Remove-Item -LiteralPath $root -Recurse -Force
}
