#!/bin/bash
# One fully automatic modeset test boot, version 2 (docs/grub_recovery.md).
# The NVIDIA driver loads from the startup image (initramfs), which holds its
# own copy of /etc/modprobe.d/nvidia-nomodeset.conf. This builds a separate
# test image without that file, adds a GRUB entry (no nomodeset) that uses it,
# and selects that entry for the next boot only. The normal startup image and
# the normal settings are left as they are. The undo job removes the test
# image and entry after the test boot.
set -e
SRC=/opt/templates/rust/src-tauri/benchmark/modeset_test
CONF=/etc/modprobe.d/nvidia-nomodeset.conf
KVER=6.8.0-149-generic
TEST_IMG=/boot/initrd-modeset-test.img
[ -e "$CONF" ] || { echo "$CONF not found, nothing done"; exit 1; }

# Build the test image with the "off" file moved aside; always put it back.
mv "$CONF" "$CONF.disabled"
trap 'mv "$CONF.disabled" "$CONF" 2>/dev/null || true' EXIT
echo "Building test startup image (takes a minute or two)..."
mkinitramfs -o "$TEST_IMG" "$KVER"
mv "$CONF.disabled" "$CONF"
trap - EXIT
if lsinitramfs "$TEST_IMG" | grep -q 'modprobe.d/nvidia-nomodeset.conf'; then
  rm -f "$TEST_IMG"; echo "test image still holds the off file, nothing armed"; exit 1
fi
lsinitramfs "$TEST_IMG" | grep -q 'dkms/nvidia-drm.ko' || { rm -f "$TEST_IMG"; echo "test image lacks nvidia-drm, nothing armed"; exit 1; }

install -m 755 "$SRC/42_modeset_test" /etc/grub.d/42_modeset_test
update-grub
grep -q "initrd-modeset-test.img" /boot/grub/grub.cfg || { echo "test entry missing from grub.cfg"; exit 1; }
install -m 755 "$SRC/modeset-test-undo.sh" /usr/local/sbin/modeset-test-undo.sh
install -m 644 "$SRC/modeset-test-undo.service" /etc/systemd/system/modeset-test-undo.service
systemctl daemon-reload
systemctl enable modeset-test-undo.service
grub-reboot "Modeset test (one boot)"
echo "automatic test boot armed (version 2)"
