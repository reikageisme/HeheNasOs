#!/usr/bin/env bash
# Run hehenasd on your Linux/WSL machine in dry-run mode to work on the UI.
# Privileged actions (mount, mkfs, ksmbd) are only logged. Open http://localhost:8080
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
mkdir -p dev/state
# Keep build artefacts on the Linux filesystem (much faster under WSL).
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/hehenas-target}"
exec cargo run --manifest-path core/Cargo.toml -- --config dev/hehenasd.dev.toml
