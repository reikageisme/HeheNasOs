#!/usr/bin/env bash
# Cross-compile rustnasd as a static musl binary.
#   TARGET=aarch64-unknown-linux-musl (default, Raspberry Pi) | x86_64-unknown-linux-musl
# The TLS stack (ring) contains C code, so cross builds use cargo-zigbuild
# (zig as the C cross compiler): pip install ziglang && cargo install cargo-zigbuild
set -euo pipefail

CORE="$(cd "$(dirname "$0")/../core" && pwd)"
TARGET="${TARGET:-aarch64-unknown-linux-musl}"

if ! command -v cargo >/dev/null; then
	echo "Rust is not installed:  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
	exit 1
fi
rustup target list --installed | grep -qx "$TARGET" || rustup target add "$TARGET"

cd "$CORE"
if cargo zigbuild --version >/dev/null 2>&1; then
	cargo zigbuild --release --target "$TARGET"
else
	echo "cargo-zigbuild not found; trying plain cargo (needs a C compiler for $TARGET)"
	cargo build --release --target "$TARGET"
fi

BIN="$CORE/target/$TARGET/release/rustnasd"
echo
ls -lh "$BIN"
command -v file >/dev/null && file "$BIN"
