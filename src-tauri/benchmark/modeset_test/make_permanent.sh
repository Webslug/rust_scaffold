#!/bin/bash
# Step 4 of docs/grub_recovery.md: turn NVIDIA kernel mode setting on for
# good, for kernel 6.8.0-149 only. Proven on the 13 November 2026 test boot.
# - removes nomodeset from GRUB_CMDLINE_LINUX_DEFAULT (backup kept)
# - renames /etc/modprobe.d/nvidia-nomodeset.conf to .disabled
# - rebuilds the 6.8.0-149 startup image without that file (old image kept)
# Older kernels keep their old startup images, which still switch mode
# setting off: they stay a known-good fallback under "Advanced options".
set -e
CONF=/etc/modprobe.d/nvidia-nomodeset.conf
KVER=6.8.0-149-generic
IMG=/boot/initrd.img-$KVER
BACKUP_DIR=/var/backups/modeset-permanent-2026-11-13

grep -q '^GRUB_CMDLINE_LINUX_DEFAULT=.*nomodeset' /etc/default/grub || { echo "nomodeset not in /etc/default/grub, nothing done"; exit 1; }
[ -e "$CONF" ] || { echo "$CONF not found, nothing done"; exit 1; }

mkdir -p "$BACKUP_DIR"
cp -a /etc/default/grub "$BACKUP_DIR/grub"
cp -a "$CONF" "$BACKUP_DIR/nvidia-nomodeset.conf"
cp -a "$IMG" "$BACKUP_DIR/initrd.img-$KVER"
echo "backups in $BACKUP_DIR"

mv "$CONF" "$CONF.disabled"
echo "Rebuilding the startup image for $KVER (takes a minute or two)..."
update-initramfs -u -k "$KVER"
if lsinitramfs "$IMG" | grep -q 'modprobe.d/nvidia-nomodeset.conf'; then
  echo "new startup image still holds the off file; restoring everything"
  mv "$CONF.disabled" "$CONF"; cp -a "$BACKUP_DIR/initrd.img-$KVER" "$IMG"; exit 1
fi
lsinitramfs "$IMG" | grep -q 'dkms/nvidia-drm.ko' || {
  echo "new startup image lacks nvidia-drm; restoring everything"
  mv "$CONF.disabled" "$CONF"; cp -a "$BACKUP_DIR/initrd.img-$KVER" "$IMG"; exit 1; }

sed -i -E '/^GRUB_CMDLINE_LINUX_DEFAULT=/ s/ ?\bnomodeset\b//' /etc/default/grub
grep -n '^GRUB_CMDLINE_LINUX_DEFAULT=' /etc/default/grub
update-grub
if awk "/vmlinuz-$KVER/" /boot/grub/grub.cfg | head -1 | grep -q nomodeset; then
  echo "grub.cfg still has nomodeset for $KVER"; exit 1
fi
echo "mode setting made permanent; reboot to use it"
