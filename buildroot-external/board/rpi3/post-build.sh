#!/bin/sh
# Buildroot post-build hook. $1 = TARGET_DIR
set -eu
TARGET_DIR="$1"
INITTAB="$TARGET_DIR/etc/inittab"

# Login prompt on HDMI as well as on the serial console.
if ! grep -qE '^tty1::' "$INITTAB"; then
	echo 'tty1::respawn:/sbin/getty -L tty1 0 vt100 # HDMI console' >> "$INITTAB"
fi

# rustnasd is supervised by init: restarted automatically if it ever exits.
# stdout/stderr go to syslog (/var/log/messages, in RAM, size-capped).
if ! grep -q rustnasd "$INITTAB"; then
	cat >> "$INITTAB" <<'EOF'
# RustNAS management daemon (web UI on :8080)
::respawn:/bin/sh -c '/usr/sbin/rustnasd --config /etc/rustnas/rustnasd.toml 2>&1 | /usr/bin/logger -t rustnasd; sleep 3'
EOF
fi

# Persistent state partition (p3 of the SD card).
mkdir -p "$TARGET_DIR/var/lib/rustnas"
if ! grep -q '/var/lib/rustnas' "$TARGET_DIR/etc/fstab"; then
	printf '/dev/mmcblk0p3\t/var/lib/rustnas\text4\tdefaults,noatime\t0\t2\n' >> "$TARGET_DIR/etc/fstab"
fi

# ksmbd keeps its config and password DB on the persistent partition.
rm -rf "$TARGET_DIR/etc/ksmbd"
ln -sfn /var/lib/rustnas/ksmbd "$TARGET_DIR/etc/ksmbd"

# Files coming from a Windows checkout may lose their executable bit.
chmod 0755 "$TARGET_DIR"/etc/init.d/S* 2>/dev/null || true
