#!/usr/bin/env bash
set -euo pipefail

artifacts_input=${1:?usage: smoke-homebrew-release.sh ARTIFACTS VERSION}
version=${2:?usage: smoke-homebrew-release.sh ARTIFACTS VERSION}

if [[ "$(uname -s)" == Linux ]]; then
  eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)"
fi

artifacts="$(cd "$artifacts_input" && pwd)"
root="$(mktemp -d)"
tap=microck/pangram-release-test

cleanup() {
  brew uninstall --force "$tap/pangram" >/dev/null 2>&1 || true
  brew untap "$tap" >/dev/null 2>&1 || true
  rm -rf "$root"
}
trap cleanup EXIT

export HOMEBREW_CACHE="$root/cache"
export HOMEBREW_NO_AUTO_UPDATE=1
export HOMEBREW_NO_ENV_HINTS=1
export HOMEBREW_NO_INSTALL_CLEANUP=1
export HOMEBREW_NO_INSTALL_FROM_API=1

brew tap-new --no-git "$tap"
formula="$(brew --repository "$tap")/Formula/pangram.rb"
cp "$artifacts/pangram.rb" "$formula"

python3 - "$formula" "$version" "$artifacts" <<'PY'
import sys
from pathlib import Path

formula = Path(sys.argv[1])
version = sys.argv[2]
artifacts = Path(sys.argv[3]).resolve().as_uri()
production = f"https://github.com/Microck/pangram-cli/releases/download/v{version}"
contents = formula.read_text(encoding="utf-8")
if contents.count(production) != 4:
    raise SystemExit("generated Homebrew formula has an unexpected release URL set")
formula.write_text(contents.replace(production, artifacts), encoding="utf-8")
PY

brew install "$tap/pangram"
brew test "$tap/pangram"
test "$("$(brew --prefix "$tap/pangram")/bin/pangram" --version)" = \
  "pangram ${version}"
