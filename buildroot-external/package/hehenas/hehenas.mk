################################################################################
#
# hehenas
#
################################################################################

HEHENAS_VERSION = 1.1.0
HEHENAS_SITE = $(BR2_EXTERNAL_HEHENAS_PATH)/package/hehenas/files
HEHENAS_SITE_METHOD = local
HEHENAS_LICENSE = MIT
HEHENAS_BOARD = $(call qstrip,$(BR2_PACKAGE_HEHENAS_BOARD))
HEHENAS_BINARY = $(BR2_EXTERNAL_HEHENAS_PATH)/../core/target/$(call qstrip,$(BR2_ARCH))-unknown-linux-musl/release/hehenasd

define HEHENAS_BUILD_CMDS
	@test -f $(HEHENAS_BINARY) || { \
		echo "hehenasd not found at $(HEHENAS_BINARY)"; \
		echo "Run scripts/build-core.sh first."; exit 1; }
endef

define HEHENAS_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 $(HEHENAS_BINARY) $(TARGET_DIR)/usr/sbin/hehenasd
	$(INSTALL) -D -m 0644 $(@D)/hehenasd-$(HEHENAS_BOARD).toml $(TARGET_DIR)/etc/hehenas/hehenasd.toml
endef

# hehenasd itself is supervised by busybox init (respawn entry added by the
# post-build script); only the SMB server has an init script.
define HEHENAS_INSTALL_INIT_SYSV
	$(INSTALL) -D -m 0755 $(@D)/S60ksmbd $(TARGET_DIR)/etc/init.d/S60ksmbd
endef

# Owner of shared files ("force user" in ksmbd) and the account the web
# server runs as under privilege separation. Locked, no shell, no home.
define HEHENAS_USERS
	nas 1000 nas 1000 * - /bin/false - HeheNAS
endef

$(eval $(generic-package))
