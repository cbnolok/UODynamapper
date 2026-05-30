# User Troubleshooting Guide

Welcome to UODynamapper! This guide is designed to help you resolve common issues when running the client for the first time.

---

## 1. Startup & File Issues

### Symptom: "Error: Too many open files" (Linux Only)
- **Error message**: `Failed to create file watcher from path ... Error { kind: Io(Os { code: 24, ... message: "Too many open files" }) }`
- **Cause**: The application is attempting to monitor too many files (hot-reloading) and has hit the Linux `inotify` limit.
- **Impact**: The application can continue running, but asset hot-reloading is disabled for that run.
- **Fix**: Increase the `max_user_instances` limit by running the following command in your terminal:
  ```bash
  su -c "echo 256 > /proc/sys/fs/inotify/max_user_instances"
  ```
---

## 2. Reporting a Bug (no feature requests for now)
If your issue isn't listed here, please provide the following when reporting a bug:
1. Your Operating System (Windows/Linux/OSX).
2. Your GPU model (e.g., NVIDIA RTX 3060).
3. The content of the `logs/log.txt` file after the crash.
