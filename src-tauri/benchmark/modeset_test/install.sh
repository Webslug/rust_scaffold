#!/bin/bash
# Arms one modeset test boot: disables the "modeset off" file and enables the
# one-time undo job. See docs/grub_recovery.md.
set -e
SRC=/opt/templates/rust/src-tauri/benchmark/modeset_test
[ -e /etc/modprobe.d/nvidia-nomodeset.conf ] || { echo "nvidia-nomodeset.conf not found, nothing done"; exit 1; }
install -m 755 "$SRC/modeset-test-undo.sh" /usr/local/sbin/modeset-test-undo.sh
install -m 644 "$SRC/modeset-test-undo.service" /etc/systemd/system/modeset-test-undo.service
systemctl daemon-reload
systemctl enable modeset-test-undo.service
mv /etc/modprobe.d/nvidia-nomodeset.conf /etc/modprobe.d/nvidia-nomodeset.conf.disabled
echo "test boot armed"
