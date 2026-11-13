#!/bin/bash
# Opened by the "GPU check" desktop launcher: runs gpu_check.py, shows the
# modeset test log if it exists, and waits so the window stays open.
python3 /opt/templates/rust/src-tauri/benchmark/gpu_check.py 2>&1 | grep -v '^src/nv_gbm'
if [ -r /var/log/modeset-test.log ]; then
  echo
  echo "--- /var/log/modeset-test.log ---"
  cat /var/log/modeset-test.log
fi
echo
read -r -p "Press Enter to close."
