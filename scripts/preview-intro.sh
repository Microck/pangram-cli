#!/usr/bin/env bash
# Plays the TUI intro in this terminal with throwaway settings and state.
# Usage: scripts/preview-intro.sh [cat-pufferfish|fox|random]
set -euo pipefail

artwork=${1:-cat-pufferfish}
case "$artwork" in
  cat-pufferfish | fox | random) ;;
  *)
    echo "usage: scripts/preview-intro.sh [cat-pufferfish|fox|random]" >&2
    exit 2
    ;;
esac

cd "$(dirname "$0")/.."
cargo build --quiet --features dev-tools --bin pangram

root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT
printf 'config_version = 1\n\n[tui]\nintro = "always"\n\n[updates]\ncheck_on_tui_start = false\n' \
  >"$root/config.toml"

# `random` leaves the pin unset so the real one-in-four draw decides.
pin=()
if [[ "$artwork" != random ]]; then
  pin=(PANGRAM_TUI_TEST_INTRO_ARTWORK="$artwork")
fi

# A synthetic key skips credential onboarding; nothing is billed unless an
# analysis is submitted. Unset CI so the intro stays eligible.
env -u CI \
  PANGRAM_CONFIG="$root/config.toml" \
  PANGRAM_DATA_DIR="$root/data" \
  PANGRAM_API_KEY=synthetic-preview-key \
  COLORTERM="${COLORTERM:-truecolor}" \
  "${pin[@]}" \
  ./target/debug/pangram
