# User Troubleshooting Guide

Welcome to UODynamapper! This guide is designed to help you resolve common issues when running the client for the first time.

---

## 1. Startup & File Issues

### Symptom: "Error: Too many open files" (Linux Only)
- **Error message**: `Failed to create file watcher from path ... Error { kind: Io(Os { code: 24, ... message: "Too many open files" }) }`
- **Cause**: The application is attempting to monitor too many files (hot-reloading) and has hit the Linux `inotify` limit.
- **Fix**: Increase the `max_user_instances` limit by running the following command in your terminal:
  ```bash
  su -c "echo 256 > /proc/sys/fs/inotify/max_user_instances"
  ```

### Symptom: Screen is Black or Crashes on Startup
- **Cause**: Your Graphics Card might not support the modern rendering engine (Vulkan, DX12, or Metal).
- **Fix**: 
  - Ensure your GPU drivers are up-to-date.
  - Check if your hardware supports at least **DirectX 11/12** or **Vulkan 1.2**.

---

## 2. Graphics & Performance

### Symptom: Menu text is too small or too big
- **Fix**: The client uses your system's scaling Factor. You can override this in the `ui.toml` settings by adjusting the `global_scale` value.

---

## 3. Reporting a Bug (no feature requests for now)
If your issue isn't listed here, please provide the following when reporting a bug:
1. Your Operating System (Windows/Linux/OSX).
2. Your GPU model (e.g., NVIDIA RTX 3060).
3. The content of the `logs/log.txt` file after the crash.
