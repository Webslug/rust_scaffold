# NVIDIA mode setting test: plan and recovery

Written 13 November 2026 for Kim's PC (ASUS Z170, UEFI, Lubuntu 22.04,
NVIDIA 535, disk `sda2` = Lubuntu, UUID `a89fc1b2-23fb-4699-8e1b-c86c38b9a68c`).
Photograph the **Step 3 card** and the **Recovery** section before rebooting.

## Where we are (13 November 2026)

- **Step 1 done and proven.** The GRUB menu now shows for 10 seconds. The new
  kernel 6.8.0-149 boots normally.
- **Step 2 done and proven.** One test boot with the "modeset off" file
  switched off. The desktop started normally, but mode setting stayed off:
  **the NVIDIA 535 driver obeys `nomodeset` on the boot line.** The GPU
  buffer test still failed. The automatic undo worked: the file was put back
  and the undo job removed itself.
- **The PC is back to its original state**, except for the visible 10-second
  menu.
- **Conclusion so far:** mode setting needs *both* switches off: the file and
  `nomodeset`. Step 3 tests exactly that, for one boot only.

## Step 3 card (photograph this)

Before the reboot, Claude arms the test boot (file switched off, one-time
undo job enabled). Then:

1. Reboot. At the GRUB menu press an **arrow key** to stop the countdown.
2. **Ubuntu** is highlighted. Press **e**.
3. Arrow down to the line starting with `linux` (it contains `quiet splash
   nomodeset`). It may wrap onto two screen lines.
4. Move the cursor onto the word `nomodeset` and delete it (and one of the
   spaces). Change nothing else.
5. Press **Ctrl+X** (or **F10**) to boot.
6. When the desktop appears, open **GPU check** at once. Wait one minute,
   open it again: it then also shows the test log.
7. Tell Claude what it says. If the GPU buffer passes, the app can be
   measured on this same boot.

**If the screen stays black:** wait **two minutes** (the undo job needs one
minute after startup), then hold the power button for 5 seconds and start
again. Let GRUB start normally. The GRUB edit is forgotten and the undo has
put the file back, so this boot is the original setting. If that does not
give a desktop, use the Recovery section below, starting at C.2.

## What was off, and why it matters

NVIDIA mode setting was switched off in two places:

1. `/etc/default/grub`: `nomodeset` on the boot line (added at install time,
   Lubuntu would not start without it).
2. `/etc/modprobe.d/nvidia-nomodeset.conf`: `options nvidia-drm modeset=0`
   (made by hand on 7 May 2026, the day driver 580 was removed and 535
   reinstalled). It overrides the driver's own file, which says `modeset=1`.

WebKit can only draw on the GPU with mode setting on (it needs GPU memory
buffers, "GBM", which NVIDIA only provides with mode setting). None of this
touches your files: the worst case is a PC that does not reach the desktop
until one file is renamed back.

## The plan, one step at a time

**Step 1, harmless (done).** Show the GRUB menu for 10 seconds at every boot.
Backup copy: `/etc/default/grub.bak-2026-11-13` (only two lines differ).

**Step 2, one test boot (done).** File switched off, `nomodeset` kept, with
an automatic undo. Result: mode setting stays off while `nomodeset` is
present.

**Step 3, one test boot (next).** File switched off again with the automatic
undo, and `nomodeset` removed at the GRUB menu for that boot only (Step 3
card above). Both changes revert by themselves.

**Step 4, only if step 3 gives a working desktop and the GPU check passes.**
Make it permanent: remove `nomodeset` from `/etc/default/grub` and leave the
file switched off. Decided separately, after step 3.

### How the automatic undo works

`install.sh` (run once with your password, before the test reboot):

- installs `/usr/local/sbin/modeset-test-undo.sh` and the one-time job
  `modeset-test-undo.service`, and enables it;
- renames `/etc/modprobe.d/nvidia-nomodeset.conf` to
  `nvidia-nomodeset.conf.disabled`.

On the test boot, 60 seconds after startup (it runs even if the login screen
fails), the job:

- writes a report to `/var/log/modeset-test.log` (kernel, boot line, whether
  mode setting was really on, whether the login screen service was running);
- renames the file back;
- disables and deletes itself.

It does not run if the machine freezes completely before that minute is up.
The scripts are kept in `src-tauri/benchmark/modeset_test/` in the project.

### Checking

The desktop launcher **GPU check** (also in the applications menu) runs
`src-tauri/benchmark/gpu_check.py`, shows the test log, and waits for Enter.
Closing the window instead is fine. It reports:

1. whether the boot line has `nomodeset`;
2. the `modeset` value the module loader applies;
3. whether a GPU buffer can be allocated. This is the deciding test.

---

## Recovery (photograph this)

### A. Get to the GRUB menu

- The menu shows by itself for 10 seconds. Press any arrow key to stop the
  countdown.
- If it does not show: restart, and as soon as the ASUS logo goes, press
  **Esc** once (UEFI). If you get a black screen with `grub>`, type
  `normal` and press Enter, then press Esc again.

### B. Edit the boot line for one boot

1. Highlight **Ubuntu** (Lubuntu is listed as Ubuntu).
2. Press **e**.
3. Use the arrow keys to reach the line that starts with `linux` and
   contains `quiet splash`. It may wrap onto two screen lines.
4. Move to the words you need to change (Delete and Backspace work as normal):
   - **To go back to the old safe setting:** make sure the line contains
     `nomodeset` (type it back after `quiet splash` if it is missing). Step 2
     proved that `nomodeset` alone keeps mode setting off.
   - **For the step 3 test:** delete the word `nomodeset`.
5. Press **Ctrl+X** (or **F10**) to boot. The change lasts for this boot only.

### C. If the screen stays black or Lubuntu will not start

Try these in order:

1. **Wait, then restart.** Wait two minutes, hold the power button for 5
   seconds, then press it again and let GRUB start normally. After a test
   boot this is normally all you need.
2. **Old kernel.** In the GRUB menu choose **Advanced options for Ubuntu**,
   then **Ubuntu, with Linux 6.8.0-138-generic**.
3. **Safe line.** GRUB edit as in B, "go back to the old safe setting".
4. **Text mode, then undo.** GRUB edit: add `systemd.unit=multi-user.target`
   at the end of the `linux` line, Ctrl+X. Log in as kim, then type (if the
   first line says "No such file", the undo already happened; carry on):

        sudo mv /etc/modprobe.d/nvidia-nomodeset.conf.disabled /etc/modprobe.d/nvidia-nomodeset.conf
        sudo cp /etc/default/grub.bak-2026-11-13 /etc/default/grub
        sudo update-grub
        sudo reboot

### D. Last resort: the Lubuntu pen drive

1. Plug in the pen drive, restart, press **F8** at the ASUS logo for the
   boot menu, and choose the USB drive (the **UEFI:** entry).
2. Choose **Try Lubuntu**. Open a terminal (QTerminal).
3. Type (the long number is your Lubuntu disk, so the drive letters do not
   matter):

        sudo mount /dev/disk/by-uuid/a89fc1b2-23fb-4699-8e1b-c86c38b9a68c /mnt
        sudo mv /mnt/etc/modprobe.d/nvidia-nomodeset.conf.disabled /mnt/etc/modprobe.d/nvidia-nomodeset.conf
        sudo umount /mnt

4. Remove the pen drive and restart. If step 4 of the plan was already done,
   also use C.3 at the GRUB menu for that boot, then run the text-mode
   commands in C.4 once you are back in Lubuntu.

If `mount` complains, type `lsblk -f` and use the 232 GB `ext4` partition
instead, for example `sudo mount /dev/sda2 /mnt`.

---

## Notes and unknowns

- **Known now:** the 535 driver obeys `nomodeset` (step 2).
- **Not known:** whether this machine shows a desktop with mode setting on.
  The 7 May history (driver 580 removed, mode setting switched off the same
  day) suggests it caused trouble then, possibly only with driver 580.
- **Not known:** whether a boot-line `nvidia-drm.modeset=` setting beats the
  file in `/etc/modprobe.d` (on this machine the loader lists the file's
  values after it). The plan does not rely on it.
- Even with GPU buffers working, WebKitGTK on NVIDIA has known problems
  (blank or flickering windows). If that happens, the app can still be run
  on the CPU path with `WEBKIT_DISABLE_DMABUF_RENDERER=1`.

## Progress log

- 13 November 2026, step 1: `/etc/default/grub` set to
  `GRUB_TIMEOUT_STYLE=menu` and `GRUB_TIMEOUT=10`, `update-grub` succeeded.
  Rebooted fine: menu shown, kernel 6.8.0-149.
- 13 November 2026, step 2 armed: file renamed (loader then applied only
  `modeset=1`), undo job enabled.
- 13 November 2026, step 2 test boot (10:29): desktop started normally.
  Runtime `/sys/module/nvidia_drm/parameters/modeset` was `N` with
  `nomodeset` on the boot line. GBM buffer failed. Undo ran at 10:30:56,
  restored the file and removed itself. Verified afterwards: file back, job
  and script gone.
- 13 November 2026, step 3: scripts recreated in
  `src-tauri/benchmark/modeset_test/` (`/tmp` was cleared by the reboot),
  same content as step 2.
- 13 November 2026, step 3 armed: file renamed to
  `nvidia-nomodeset.conf.disabled`, undo job enabled, undo script verified
  identical to the project copy. `/etc/default/grub` unchanged (still has
  `nomodeset`). Waiting for the test boot with the GRUB edit.
- 13 November 2026, step 3 attempt (boot 10:38): the GRUB edit did not take
  effect, the boot line still had `nomodeset`. Same result as step 2
  (runtime modeset `N`, GBM failed, desktop fine). Undo ran at 10:40:05 and
  restored the file. Step 3 still to be done; it must be armed again first.
- 13 November 2026: step 3 armed again.
- 13 November 2026, automatic test boot (10:57, GRUB entry without
  `nomodeset` chosen once with `grub-reboot`): **desktop started normally
  without `nomodeset`.** But runtime modeset was still `N` and GBM failed.
  Cause found (read-only root check): the startup image
  `/boot/initrd.img-6.8.0-149-generic` contains `nvidia-drm.ko` and its own
  copy of `/etc/modprobe.d/nvidia-nomodeset.conf`, so the driver loads early
  with `modeset=0` before the renamed file on disk is ever read. Undo ran at
  10:58:49; system back to normal, test entry removed.
- Next: `src-tauri/benchmark/modeset_test/install_auto2.sh` builds a
  separate test startup image `/boot/initrd-modeset-test.img` without the
  "off" file, points the one-boot test entry at it, and arms the undo (which
  deletes the test image and entry). The normal startup image and settings
  are not changed.
