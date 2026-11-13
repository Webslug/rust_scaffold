#!/usr/bin/env python3
"""Checks whether WebKitGTK could render on the NVIDIA GPU.

Prints three facts and a verdict:
  1. whether the kernel boot line contains nomodeset,
  2. the nvidia-drm options the module loader applies (last value wins),
  3. whether a GBM buffer can be allocated on the GPU's render node, which is
     what WebKitGTK's DMA-BUF renderer needs. This is the deciding test.

No root needed. Changes nothing. See docs/grub_recovery.md and
docs/gpu_rendering.md.
"""
import ctypes
import glob
import os
import subprocess
import time

DEBUG_MODE = False
LOG_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "gpu_check.log")

GBM_FORMAT_ARGB8888 = 0x34325241
GBM_BO_USE_RENDERING = 1 << 2


def log(message):
    if DEBUG_MODE:
        with open(LOG_FILE, "a", encoding="utf-8") as handle:
            handle.write(f"[{time.strftime('%Y-%m-%d %H:%M:%S')}] {message}\n")


def boot_line():
    with open("/proc/cmdline", encoding="ascii") as handle:
        return handle.read().strip()


def loader_modeset():
    """The modeset value modprobe would pass to nvidia-drm, or None."""
    try:
        out = subprocess.run(["modprobe", "--show-depends", "nvidia-drm"],
                             capture_output=True, text=True, timeout=10).stdout
    except (OSError, subprocess.TimeoutExpired) as exc:
        log(f"modprobe failed: {exc}")
        return None, ""
    line = next((l for l in out.splitlines() if "nvidia-drm.ko" in l), "")
    values = [part.split("=", 1)[1] for part in line.split() if part.startswith("modeset=")]
    return (values[-1] if values else None), line.strip()


def gbm_buffer():
    """(ok, detail). ok is True when a GPU buffer could be allocated."""
    nodes = sorted(glob.glob("/dev/dri/renderD*"))
    if not nodes:
        return False, "no /dev/dri/renderD* device"
    try:
        gbm = ctypes.CDLL("libgbm.so.1")
    except OSError as exc:
        return False, f"libgbm not found: {exc}"
    gbm.gbm_create_device.restype = ctypes.c_void_p
    gbm.gbm_create_device.argtypes = [ctypes.c_int]
    gbm.gbm_device_get_backend_name.restype = ctypes.c_char_p
    gbm.gbm_device_get_backend_name.argtypes = [ctypes.c_void_p]
    gbm.gbm_bo_create.restype = ctypes.c_void_p
    gbm.gbm_bo_create.argtypes = [ctypes.c_void_p] + [ctypes.c_uint32] * 4
    gbm.gbm_bo_destroy.argtypes = [ctypes.c_void_p]
    gbm.gbm_device_destroy.argtypes = [ctypes.c_void_p]
    results = []
    for node in nodes:
        try:
            fd = os.open(node, os.O_RDWR)
        except OSError as exc:
            results.append((False, f"{node}: cannot open ({exc})"))
            continue
        try:
            dev = gbm.gbm_create_device(fd)
            if not dev:
                results.append((False, f"{node}: no GBM device"))
                continue
            backend = (gbm.gbm_device_get_backend_name(dev) or b"?").decode()
            bo = gbm.gbm_bo_create(dev, 256, 256, GBM_FORMAT_ARGB8888, GBM_BO_USE_RENDERING)
            if bo:
                gbm.gbm_bo_destroy(bo)
            gbm.gbm_device_destroy(dev)
            results.append((bool(bo), f"{node}: backend {backend}, buffer {'OK' if bo else 'FAILED'}"))
        finally:
            os.close(fd)
    ok = any(r[0] for r in results)
    return ok, "; ".join(r[1] for r in results)


def main():
    line = boot_line()
    nomodeset = "nomodeset" in line.split()
    value, loader_line = loader_modeset()
    ok, detail = gbm_buffer()
    log(f"cmdline={line!r} loader={loader_line!r} gbm={detail!r}")

    print(f"1. Boot line has nomodeset:     {'yes' if nomodeset else 'no'}")
    print(f"2. Loader applies modeset:      {value if value is not None else 'unknown'}"
          f"   ({loader_line or 'modprobe gave no line'})")
    print(f"3. GPU buffer (GBM) allocation: {'OK' if ok else 'FAILED'}   ({detail})")
    print()
    if ok:
        print("Verdict: GPU buffers work. Next: open webkit://gpu in "
              "/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/MiniBrowser and check the Policy.")
    else:
        print("Verdict: WebKitGTK cannot use the GPU yet (mode setting is still off).")


if __name__ == "__main__":
    main()
