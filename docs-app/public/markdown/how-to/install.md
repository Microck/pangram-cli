# Install Pangram CLI

These commands become live when the first public release is published. Until
then, clone the repository and build the locked Rust package for development.

## macOS or Linux

Download the versioned installer, inspect it, and run it:

```bash
curl --fail --location --silent --show-error \
  https://github.com/Microck/pangram-cli/releases/latest/download/pangram-installer.sh \
  --output pangram-installer.sh
less pangram-installer.sh
sh pangram-installer.sh
```

The default executable path is `$HOME/.local/bin/pangram`.

## Windows

Download the versioned installer, inspect it, and run it in PowerShell:

```powershell
Invoke-WebRequest `
  https://github.com/Microck/pangram-cli/releases/latest/download/pangram-installer.ps1 `
  -OutFile pangram-installer.ps1
Get-Content .\pangram-installer.ps1
.\pangram-installer.ps1
```

The default executable path is
`%LOCALAPPDATA%\Programs\Pangram\bin\pangram.exe`.

The installers verify the release archive before installation, then the
downloaded Pangram binary verifies the signed manifest and complete archive.
They do not edit PATH. Follow the exact PATH instruction they print when the
install directory is not already available.
