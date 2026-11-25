#!/usr/bin/env bash
# Buildroot post-image hook: A/B SD-card image + update bundle.
# Runs from the Buildroot top directory with BINARIES_DIR, BUILD_DIR,
# HOST_DIR, TARGET_DIR and BR2_CONFIG exported.
set -euo pipefail

BOARD_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$BOARD_DIR/../../.." && pwd)"
GENIMAGE_CFG="${BUILD_DIR}/genimage-rustnas.cfg"
FW="${BINARIES_DIR}/rpi-firmware"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO_DIR/core/Cargo.toml" | head -n1)

# gpu_mem=16 only boots with the cut-down firmware (start_cd.elf).
if ! grep -q '^BR2_PACKAGE_RPI_FIRMWARE_CD=y' "${BR2_CONFIG}"; then
	echo "post-image: cut-down firmware not selected -> gpu_mem=64"
	sed -i 's/^gpu_mem=16$/gpu_mem=64/' "$FW/config.txt"
fi

# --- A/B boot files: the slot is chosen by slot.txt (included from config.txt)
sed -i '/^kernel=/d; /^include slot.txt/d' "$FW/config.txt"
printf '\n# A/B slot selection (rewritten by updates)\ninclude slot.txt\n' >> "$FW/config.txt"
BASE=$(sed 's#root=[^ ]*##; s#rustnas\.slot=[ab]##' "$FW/cmdline.txt" | tr -s ' ')
printf 'root=/dev/mmcblk0p2 %s rustnas.slot=a\n' "$BASE" | tr -s ' ' > "$FW/cmdline-a.txt"
printf 'root=/dev/mmcblk0p3 %s rustnas.slot=b\n' "$BASE" | tr -s ' ' > "$FW/cmdline-b.txt"
printf 'kernel=Image-a\ncmdline=cmdline-a.txt\n' > "$FW/slot.txt"
printf 'a = "%s"\nb = "%s"\n' "$VERSION" "$VERSION" > "$FW/rustnas-slots.toml"
cp "${BINARIES_DIR}/Image" "${BINARIES_DIR}/Image-a"
cp "${BINARIES_DIR}/Image" "${BINARIES_DIR}/Image-b"

# --- persistent state partition, pre-seeded with the default /etc files
SEED="${BUILD_DIR}/persist-seed"
rm -rf "$SEED"
mkdir -p "$SEED/etc" "$SEED/ksmbd"
cp -a "${TARGET_DIR}/usr/share/rustnas/etc-default/." "$SEED/etc/"
PERSIST="${BINARIES_DIR}/persist.ext4"
rm -f "$PERSIST"
truncate -s 128M "$PERSIST"
"${HOST_DIR}/sbin/mkfs.ext4" -q -F -L rustnas-data -d "$SEED" "$PERSIST"

# --- FAT boot partition contents
FILES=()
for f in "${BINARIES_DIR}"/*.dtb "$FW"/* "${BINARIES_DIR}/Image-a" "${BINARIES_DIR}/Image-b"; do
	[ -e "$f" ] || continue
	FILES+=("${f#"${BINARIES_DIR}"/}")
done
BOOT_FILES=$(printf '\\t\\t\\t"%s",\\n' "${FILES[@]}")
sed "s|#BOOT_FILES#|${BOOT_FILES}|" "$BOARD_DIR/genimage.cfg.in" > "$GENIMAGE_CFG"
support/scripts/genimage.sh -c "$GENIMAGE_CFG"

# --- update bundle for the web UI: manifest.json first, then Image + rootfs
UPD="${BUILD_DIR}/update-bundle"
rm -rf "$UPD"
mkdir -p "$UPD"
cp "${BINARIES_DIR}/Image" "$UPD/Image"
gzip -9 -c "${BINARIES_DIR}/rootfs.ext4" > "$UPD/rootfs.ext4.gz"
cat > "$UPD/manifest.json" <<EOF
{
  "version": "$VERSION",
  "board": "rpi3b",
  "kernel_sha256": "$(sha256sum "$UPD/Image" | cut -d' ' -f1)",
  "rootfs_sha256": "$(sha256sum "$UPD/rootfs.ext4.gz" | cut -d' ' -f1)"
}
EOF
tar --format=ustar -C "$UPD" -cf "${BINARIES_DIR}/rustnas-rpi3b-update-${VERSION}.tar" manifest.json Image rootfs.ext4.gz
echo "post-image: ${BINARIES_DIR}/rustnas-rpi3b-aarch64.img"
echo "post-image: ${BINARIES_DIR}/rustnas-rpi3b-update-${VERSION}.tar"
