#!/bin/bash
# Arms one fully automatic modeset test boot: a GRUB entry without nomodeset,
# chosen for the next boot only (grub-reboot), plus the "modeset off" file
# switched off and the one-time undo job. See docs/grub_recovery.md.
set -e
SRC=/opt/templates/rust/src-tauri/benchmark/modeset_test
[ -e /etc/modprobe.d/nvidia-nomodeset.conf ] || { echo "nvidia-nomodeset.conf not found, nothing done"; exit 1; }
install -m 755 "$SRC/42_modeset_test" /etc/grub.d/42_modeset_test
update-grub
grep -q "Modeset test (one boot)" /boot/grub/grub.cfg || { echo "test entry missing from grub.cfg"; exit 1; }
install -m 755 "$SRC/modeset-test-undo.sh" /usr/local/sbin/modeset-test-undo.sh
install -m 644 "$SRC/modeset-test-undo.service" /etc/systemd/system/modeset-test-undo.service
systemctl daemon-reload
systemctl enable modeset-test-undo.service
mv /etc/modprobe.d/nvidia-nomodeset.conf /etc/modprobe.d/nvidia-nomodeset.conf.disabled
grub-reboot "Modeset test (one boot)"
echo "automatic test boot armed"
