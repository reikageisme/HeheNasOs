#!/usr/bin/env bash
# Build the RustNAS SD-card image for Raspberry Pi 3B.
# Run on Linux or WSL2 (Ubuntu). First build: 1-3 hours, ~15 GB of disk.
#
#   scripts/build-image.sh            full build
#   scripts/build-image.sh menuconfig any extra args are passed to Buildroot's make
#
# Env: RUSTNAS_WORK (default ~/rustnas-build), BUILDROOT_VERSION (git branch/tag)
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${RUSTNAS_WORK:-$HOME/rustnas-build}"
BR_VERSION="${BUILDROOT_VERSION:-2025.02.x}"
BR_GIT="${BUILDROOT_GIT:-https://github.com/buildroot/buildroot.git}"
BR_DIR="$WORK/buildroot"
OUT="$WORK/output"

# ---- 0. host tools -----------------------------------------------------------
missing=()
for t in make gcc g++ git rsync bc cpio unzip wget file perl python3 patch xz; do
	command -v "$t" >/dev/null || missing+=("$t")
done
if [ ${#missing[@]} -gt 0 ]; then
	echo "Missing host tools: ${missing[*]}"
	echo "On Ubuntu: sudo apt install build-essential git rsync bc cpio unzip wget file perl python3 patch xz-utils libncurses-dev"
	exit 1
fi
mem_mb=$(awk '/MemTotal/ {print int($2/1024)}' /proc/meminfo)
if [ "$mem_mb" -lt 3000 ]; then
	echo "WARNING: only ${mem_mb} MB RAM. Building GCC may fail or be very slow."
	echo "         On WSL2 raise it in %UserProfile%\\.wslconfig ([wsl2] memory=6GB) then 'wsl --shutdown'."
fi

# ---- 1. sources on a Linux filesystem ------------------------------------------
# Buildroot is very slow (and breaks on permissions) on /mnt/c, /mnt/d ...
SRC="$REPO"
case "$REPO" in
	/mnt/*)
		SRC="$WORK/src"
		mkdir -p "$SRC"
		rsync -a --delete --exclude core/target --exclude .git --exclude dist --exclude dev/state "$REPO/" "$SRC/"
		;;
esac
# A Windows checkout may have CRLF line endings and no executable bits.
find "$SRC/buildroot-external" "$SRC/scripts" -type f -exec sed -i 's/\r$//' {} +
find "$SRC/scripts" "$SRC/buildroot-external" -type f \( -name '*.sh' -o -name 'S[0-9][0-9]*' \) -exec chmod 0755 {} +
EXT="$SRC/buildroot-external"

# ---- 2. rustnasd ------------------------------------------------------------
bash "$SRC/scripts/build-core.sh"

# ---- 3. Buildroot ------------------------------------------------------------
if [ ! -d "$BR_DIR/.git" ]; then
	git clone --depth 1 --branch "$BR_VERSION" "$BR_GIT" "$BR_DIR"
fi

if [ ! -f "$OUT/.config" ]; then
	make -C "$BR_DIR" O="$OUT" BR2_EXTERNAL="$EXT" raspberrypi3_64_defconfig
	"$BR_DIR/support/kconfig/merge_config.sh" -m -O "$OUT" "$OUT/.config" "$EXT/configs/rustnas_rpi3.fragment"
	make -C "$BR_DIR" O="$OUT" olddefconfig
	for sym in BR2_TOOLCHAIN_USES_MUSL BR2_PACKAGE_KSMBD_TOOLS BR2_PACKAGE_RUSTNAS BR2_PACKAGE_RPI_FIRMWARE_CD; do
		grep -q "^${sym}=y" "$OUT/.config" || echo "WARNING: $sym is not enabled in $OUT/.config"
	done
fi

if [ $# -gt 0 ]; then
	exec make -C "$BR_DIR" O="$OUT" "$@"
fi

# rustnasd is a prebuilt binary: force a reinstall so changes are picked up.
make -C "$BR_DIR" O="$OUT" rustnas-dirclean >/dev/null
make -C "$BR_DIR" O="$OUT"

# ---- 4. result ---------------------------------------------------------------
IMG="$OUT/images/rustnas-rpi3b-aarch64.img"
mkdir -p "$REPO/dist"
xz -T0 -9 -c "$IMG" > "$REPO/dist/rustnas-rpi3b-aarch64.img.xz"
echo
echo "Done: $REPO/dist/rustnas-rpi3b-aarch64.img.xz"
echo "Flash it with Raspberry Pi Imager ('Use custom') or balenaEtcher."
