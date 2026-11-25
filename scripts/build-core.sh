#!/usr/bin/env bash
# Cross-compile hehenasd as a static musl binary.
#   TARGET=aarch64-unknown-linux-musl (default, Raspberry Pi) | x86_64-unknown-linux-musl
# The TLS stack (ring) contains C/asm, so an aarch64 C compiler is needed:
#   Ubuntu/Debian: sudo apt install gcc-aarch64-linux-gnu
# Linking is done by rust-lld (see core/.cargo/config.toml), against Rust's
# bundled musl, so no musl toolchain is required.
set -euo pipefail

CORE="$(cd "$(dirname "$0")/../core" && pwd)"
TARGET="${TARGET:-aarch64-unknown-linux-musl}"

if ! command -v cargo >/dev/null; then
	echo "Rust is not installed:  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
	exit 1
fi
rustup target list --installed | grep -qx "$TARGET" || rustup target add "$TARGET"

case "$TARGET" in
	aarch64-unknown-linux-musl)
		if ! command -v aarch64-linux-gnu-gcc >/dev/null; then
			echo "Missing aarch64 C compiler: sudo apt install gcc-aarch64-linux-gnu"
			exit 1
		fi
		export CC_aarch64_unknown_linux_musl=aarch64-linux-gnu-gcc
		export AR_aarch64_unknown_linux_musl=aarch64-linux-gnu-ar
		;;
	x86_64-unknown-linux-musl)
		export CC_x86_64_unknown_linux_musl="${CC_x86_64_unknown_linux_musl:-gcc}"
		;;
esac

cd "$CORE"
cargo build --release --target "$TARGET"

BIN="$CORE/target/$TARGET/release/hehenasd"
echo
ls -lh "$BIN"
command -v file >/dev/null && file "$BIN"
