################################################################################
#
# rustnas
#
################################################################################

RUSTNAS_VERSION = 0.1.0
RUSTNAS_SITE = $(BR2_EXTERNAL_RUSTNAS_PATH)/package/rustnas/files
RUSTNAS_SITE_METHOD = local
RUSTNAS_LICENSE = MIT
RUSTNAS_BINARY = $(call qstrip,$(BR2_PACKAGE_RUSTNAS_BINARY))

define RUSTNAS_BUILD_CMDS
	@test -f $(RUSTNAS_BINARY) || { \
		echo "rustnasd not found at $(RUSTNAS_BINARY)"; \
		echo "Run scripts/build-core.sh first."; exit 1; }
endef

define RUSTNAS_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 $(RUSTNAS_BINARY) $(TARGET_DIR)/usr/sbin/rustnasd
	$(INSTALL) -D -m 0644 $(@D)/rustnasd.toml $(TARGET_DIR)/etc/rustnas/rustnasd.toml
endef

# rustnasd itself is supervised by busybox init (respawn entry added by the
# board post-build script); only the SMB server has an init script.
define RUSTNAS_INSTALL_INIT_SYSV
	$(INSTALL) -D -m 0755 $(@D)/S60ksmbd $(TARGET_DIR)/etc/init.d/S60ksmbd
endef

# Owner of shared files ("force user" in ksmbd). Locked, no shell, no home.
define RUSTNAS_USERS
	nas 1000 nas 1000 * - /bin/false - RustNAS share owner
endef

$(eval $(generic-package))
