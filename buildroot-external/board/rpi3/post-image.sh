#!/usr/bin/env bash
# Buildroot post-image hook: assemble rustnas-rpi3b-aarch64.img
# Runs from the Buildroot top directory with BINARIES_DIR, BUILD_DIR,
# HOST_DIR and BR2_CONFIG exported.
set -euo pipefail

BOARD_DIR="$(dirname "$0")"
GENIMAGE_CFG="${BUILD_DIR}/genimage-rustnas.cfg"
FW_CFG="${BINARIES_DIR}/rpi-firmware/config.txt"

# gpu_mem=16 only boots with the cut-down firmware (start_cd.elf).
if ! grep -q '^BR2_PACKAGE_RPI_FIRMWARE_CD=y' "${BR2_CONFIG}"; then
	echo "post-image: cut-down firmware not selected -> gpu_mem=64"
	sed -i 's/^gpu_mem=16$/gpu_mem=64/' "${FW_CFG}"
fi

# Empty ext4 filesystem for the persistent state partition.
PERSIST="${BINARIES_DIR}/persist.ext4"
rm -f "${PERSIST}"
truncate -s 128M "${PERSIST}"
"${HOST_DIR}/sbin/mkfs.ext4" -q -F -L rustnas-data "${PERSIST}"

# Everything the firmware needs on the FAT partition.
FILES=()
for f in "${BINARIES_DIR}"/*.dtb "${BINARIES_DIR}"/rpi-firmware/*; do
	[ -e "$f" ] || continue
	FILES+=("${f#"${BINARIES_DIR}"/}")
done
KERNEL=$(sed -n 's/^kernel=//p' "${FW_CFG}")
FILES+=("${KERNEL}")
BOOT_FILES=$(printf '\\t\\t\\t"%s",\\n' "${FILES[@]}")
sed "s|#BOOT_FILES#|${BOOT_FILES}|" "${BOARD_DIR}/genimage.cfg.in" > "${GENIMAGE_CFG}"

support/scripts/genimage.sh -c "${GENIMAGE_CFG}"
echo "post-image: ${BINARIES_DIR}/rustnas-rpi3b-aarch64.img"
