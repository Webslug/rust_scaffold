#!/bin/bash
# One-time undo for the NVIDIA modeset test (docs/grub_recovery.md).
# Records what the test boot did, restores the "modeset off" file, removes itself.
LOG=/var/log/modeset-test.log
{
  echo "=== $(date '+%Y-%m-%d %H:%M:%S') test boot report"
  echo "kernel:  $(uname -r)"
  echo "cmdline: $(cat /proc/cmdline)"
  echo "nvidia_drm modeset (runtime): $(cat /sys/module/nvidia_drm/parameters/modeset 2>&1)"
  echo "display manager: $(systemctl is-active display-manager.service)"
} >> "$LOG"
if [ -e /etc/modprobe.d/nvidia-nomodeset.conf.disabled ]; then
  mv /etc/modprobe.d/nvidia-nomodeset.conf.disabled /etc/modprobe.d/nvidia-nomodeset.conf
  echo "restored /etc/modprobe.d/nvidia-nomodeset.conf" >> "$LOG"
fi
if [ -e /boot/initrd-modeset-test.img ]; then
  rm -f /boot/initrd-modeset-test.img && echo "removed test startup image" >> "$LOG"
fi
if [ -e /etc/grub.d/42_modeset_test ]; then
  rm -f /etc/grub.d/42_modeset_test
  update-grub >/dev/null 2>&1 && echo "removed GRUB test entry" >> "$LOG"
fi
systemctl disable modeset-test-undo.service >> "$LOG" 2>&1
rm -f /etc/systemd/system/modeset-test-undo.service
systemctl daemon-reload
echo "undo job removed" >> "$LOG"
rm -f /usr/local/sbin/modeset-test-undo.sh
