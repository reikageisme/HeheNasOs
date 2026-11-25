#!/usr/bin/env bash
# Cross-compile rustnasd as a static ARM64 binary for the Raspberry Pi 3B.
# Needs only rustup (rust-lld does the linking; all deps are pure Rust).
set -euo pipefail

CORE="$(cd "$(dirname "$0")/../core" && pwd)"
TARGET=aarch64-unknown-linux-musl

if ! command -v cargo >/dev/null; then
	echo "Rust is not installed. Install it with:"
	echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
	exit 1
fi
rustup target list --installed | grep -qx "$TARGET" || rustup target add "$TARGET"

cd "$CORE"
cargo build --release --target "$TARGET"

BIN="$CORE/target/$TARGET/release/rustnasd"
echo
ls -lh "$BIN"
command -v file >/dev/null && file "$BIN"
