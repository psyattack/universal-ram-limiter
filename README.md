> [!CAUTION]
> ### ⚠️ Project Archived
>
> Development has been moved to **[ProcessHandler](https://github.com/psyattack/ProcessHandler)**.  
> This repository is archived and no longer maintained. Please use **ProcessHandler** instead.

# RAM Limiter

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Platform](https://img.shields.io/badge/platform-Windows-blue.svg)
![Status](https://img.shields.io/badge/status-archived-lightgrey.svg)

---

## 📋 Overview

**RAM Limiter** is a legacy, lightweight Windows utility written in Rust with native Win32 controls (`native-windows-gui`). It periodically trims or caps the working set memory of selected Windows processes via `SetProcessWorkingSetSizeEx`.

### Core Capabilities

- **Hard Trim & Soft Limit** — Trims process working sets to their minimum or caps memory at a custom MB ceiling (`soft_limit_mb`).
- **Skip Focused Window** — Automatically avoids trimming the foreground process to prevent stuttering in active games or apps.
- **Process Triggers** — Gates memory limiting to only run while specific target processes (e.g. games) are active.
- **Process Picker & Icon Extraction** — Real-time process memory viewer and native Win32 icon extraction.
- **Telemetry** — Cumulative page faults tracking and estimated disk write / paging statistics.
- **Profiles** — Save and switch between named presets (Gaming, Office, Idle).

---

## ⚠️ Disclaimer & Disk Wear Note

Periodically forcing Windows to trim working set memory moves memory pages into the Windows pagefile (`pagefile.sys`), freeing physical RAM at the expense of disk I/O. For modern process and memory management, please consider using **[ProcessHandler](https://github.com/psyattack/ProcessHandler)** instead.

---

## 🛠️ Build from Source (Legacy)

```sh
git clone https://github.com/psyattack/universal-ram-limiter.git
cd universal-ram-limiter
cargo build --release
```

The compiled binary will be located at `target/release/ram-limiter.exe`.

---

## 📄 License

This project is licensed under the [MIT License](LICENSE).
