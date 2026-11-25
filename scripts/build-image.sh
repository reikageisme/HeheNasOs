#!/usr/bin/env bash
# Build a RustNAS OS image. Run on Linux or WSL2 (Ubuntu); GitHub Actions does
# the same (see .github/workflows/build.yml).
#
#   BOARD=rpi3   scripts/build-image.sh     SD card image + A/B update bundle (default)
#   BOARD=x86_64 scripts/build-image.sh     live ISO for PCs
#   scripts/build-image.sh menuconfig       extra args go to Buildroot's make
#
# Env: RUSTNAS_WORK (default ~/rustnas-build), BUILDROOT_VERSION (git branch/tag)
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
BOARD="${BOARD:-rpi3}"
WORK="${RUSTNAS_WORK:-$HOME/rustnas-build}"
BR_VERSION="${BUILDROOT_VERSION:-2025.02.x}"
BR_GIT="${BUILDROOT_GIT:-https://github.com/buildroot/buildroot.git}"
BR_DIR="$WORK/buildroot"
OUT="$WORK/output-$BOARD"

case "$BOARD" in
	rpi3) BASE=raspberrypi3_64_defconfig; FRAG=rustnas_rpi3.fragment; RTARGET=aarch64-unknown-linux-musl ;;
	x86_64) BASE=pc_x86_64_bios_defconfig; FRAG=rustnas_x86_64.fragment; RTARGET=x86_64-unknown-linux-musl ;;
	*) echo "Unknown BOARD=$BOARD (rpi3 | x86_64)"; exit 1 ;;
esac

# ---- 0. host tools -----------------------------------------------------------
missing=()
for t in make gcc g++ git rsync bc cpio unzip wget file perl python3 patch xz gzip sha256sum; do
	command -v "$t" >/dev/null || missing+=("$t")
done
if [ ${#missing[@]} -gt 0 ]; then
	echo "Missing host tools: ${missing[*]}"
	echo "On Ubuntu: sudo apt install build-essential git rsync bc cpio unzip wget file perl python3 patch xz-utils libncurses-dev"
	exit 1
fi
mem_mb=$(awk '/MemTotal/ {print int($2/1024)}' /proc/meminfo)
if [ "$mem_mb" -lt 3000 ]; then
	echo "WARNING: only ${mem_mb} MB RAM. The build may fail or be very slow."
	echo "         On WSL2 raise it in %UserProfile%\\.wslconfig ([wsl2] memory=6GB) then 'wsl --shutdown'."
fi

# ---- 1. sources on a Linux filesystem ------------------------------------------
SRC="$REPO"
case "$REPO" in
	/mnt/*)
		SRC="$WORK/src"
		mkdir -p "$SRC"
		rsync -a --delete --exclude core/target --exclude .git --exclude dist --exclude dev/state "$REPO/" "$SRC/"
		;;
esac
find "$SRC/buildroot-external" "$SRC/scripts" -type f -exec sed -i 's/\r$//' {} +
find "$SRC/scripts" "$SRC/buildroot-external" -type f \( -name '*.sh' -o -name 'S[0-9][0-9]*' \) -exec chmod 0755 {} +
EXT="$SRC/buildroot-external"

# ---- 2. rustnasd ------------------------------------------------------------
TARGET="$RTARGET" bash "$SRC/scripts/build-core.sh"

# ---- 3. Buildroot ------------------------------------------------------------
if [ ! -d "$BR_DIR/.git" ]; then
	git clone --depth 1 --branch "$BR_VERSION" "$BR_GIT" "$BR_DIR"
fi

if [ ! -f "$OUT/.config" ]; then
	make -C "$BR_DIR" O="$OUT" BR2_EXTERNAL="$EXT" "$BASE"
	"$BR_DIR/support/kconfig/merge_config.sh" -m -O "$OUT" "$OUT/.config" "$EXT/configs/$FRAG"
	make -C "$BR_DIR" O="$OUT" olddefconfig
	for sym in BR2_TOOLCHAIN_USES_MUSL BR2_PACKAGE_KSMBD_TOOLS BR2_PACKAGE_RUSTNAS BR2_PACKAGE_NFS_UTILS \
		BR2_PACKAGE_NFTABLES BR2_PACKAGE_SMARTMONTOOLS BR2_PACKAGE_UTIL_LINUX_FDISKS BR2_PACKAGE_UTIL_LINUX_WIPEFS; do
		grep -q "^${sym}=y" "$OUT/.config" || echo "WARNING: $sym is not enabled in $OUT/.config"
	done
	[ "$BOARD" = rpi3 ] && { grep -q '^BR2_PACKAGE_RPI_FIRMWARE_CD=y' "$OUT/.config" || echo "WARNING: cut-down firmware not enabled (gpu_mem falls back to 64)"; }
fi

if [ $# -gt 0 ]; then
	exec make -C "$BR_DIR" O="$OUT" "$@"
fi

# rustnasd is a prebuilt binary: force a reinstall so changes are picked up.
make -C "$BR_DIR" O="$OUT" rustnas-dirclean >/dev/null
make -C "$BR_DIR" O="$OUT"

# ---- 4. results --------------------------------------------------------------
mkdir -p "$REPO/dist"
if [ "$BOARD" = rpi3 ]; then
	xz -T0 -9 -c "$OUT/images/rustnas-rpi3b-aarch64.img" > "$REPO/dist/rustnas-rpi3b-aarch64.img.xz"
	cp "$OUT"/images/rustnas-rpi3b-update-*.tar "$REPO/dist/"
else
	cp "$OUT/images/rootfs.iso9660" "$REPO/dist/rustnas-x86_64.iso"
fi
echo
ls -lh "$REPO/dist"
