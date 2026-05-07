# RAM Limiter

[Русская версия](README.ru.md) | English

A lightweight Windows utility that reduces RAM usage of any application by periodically trimming their working set memory. Built with Rust for minimal resource overhead.

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Platform](https://img.shields.io/badge/platform-Windows-blue.svg)

## 📋 Overview

RAM Limiter helps you manage memory usage on Windows by automatically calling `SetProcessWorkingSetSize` on selected applications. This forces Windows to trim the working set of target processes, moving unused memory pages to disk and freeing up physical RAM.

**Use cases:**
- Reduce RAM usage of memory-hungry applications (Browsers, Discord, Music)
- Keep more applications running simultaneously on systems with limited RAM
- Prevent memory leaks from consuming all available memory
- Optimize performance on low-memory systems

## ⚠️ Disclaimer

**IMPORTANT:** Applications subjected to RAM Limiter may experience instability or performance degradation. This tool forces Windows to trim process memory, which can cause:

- Increased disk I/O as memory pages are swapped
- Temporary freezes or stuttering when trimmed memory is accessed again
- Potential crashes in applications that don't handle memory pressure well
- Reduced performance in memory-intensive workloads

**Use at your own risk.** Test thoroughly with non-critical applications first. The authors are not responsible for any data loss, application crashes, or system instability resulting from the use of this software.

## ✨ Features

- **Simple GUI** — Add applications by browsing for .exe files or picking from running processes
- **Visual feedback** — See real-time RAM usage for monitored applications
- **Per-app control** — Enable/disable limiting for each application individually
- **Custom trim interval** — Tune how often the worker runs (100 ms – 600 000 ms)
- **Master on/off switch** — Pause every limiter at once without touching individual apps
- **Skip focused windows** — Leave the foreground process alone to avoid stutter during active use
- **Soft limit mode** — Optionally cap a process working set at a fixed size (e.g. 500 MB) instead of trimming to the minimum
- **Trigger processes** — Only act while one of a list of "important" apps is running
- **Page-fault & disk-write stats** — See cumulative page-fault counts and an estimate of bytes pushed into the page file
- **Profiles / presets** — Save and reload named configurations for different scenarios (gaming, idle, work, …)
- **Icon extraction** — Automatically displays application icons in the list
- **Autostart** — Optional Windows startup integration (runs minimized to tray)
- **Persistent config** — Settings saved to `%APPDATA%\RamLimiter\config.json`
- **Extremely lightweight** — Uses only ~2 MB of RAM, executable size ~0.5 MB

> **Note on UI:** The interface is intentionally minimalistic and built with native Win32 controls for maximum performance and minimal resource usage. The focus is on functionality and efficiency rather than visual aesthetics.

## 🚀 Installation

### Pre-built Binary

1. Download the latest release from the [Releases](../../releases) page
2. Extract `ram-limiter.exe` to any folder
3. Run `ram-limiter.exe`

### Build from Source

**Requirements:**
- Rust 1.70+ (install from [rustup.rs](https://rustup.rs))
- Windows 10/11

**Steps:**
```sh
git clone https://github.com/yourusername/universal-ram-limiter.git
cd universal-ram-limiter
cargo build --release
```

The compiled binary will be at `target/release/ram-limiter.exe`.

## 📖 Usage

### Adding Applications

1. **Browse for executable:**
   - Click "Browse .exe…"
   - Select the application's .exe file
   - The app will be added to the list with its icon

2. **Pick from running processes:**
   - Click "Pick running process…"
   - Select a process from the list (sorted by RAM usage)
   - Click "Add Selected"

### Managing Applications

- **Enable/Disable:** Double-click an application in the list to toggle limiting
- **Configure / remove:** Right-click a row to open the context menu (toggle, set/clear soft limit, remove)
- **View RAM usage:** The "RAM" column shows the current working-set size for the app
- **Page-fault counter:** The "Page faults" column shows the cumulative page-fault count for each monitored app

### Master switch & global settings

The toolbar exposes the limiter-wide settings:

- **Master enabled** — When unchecked, no trimming or capping happens, no matter what individual apps say. The status line shows `master switch OFF` so it's obvious limiting is paused.
- **Skip focused** — When checked, the limiter leaves the process owning the foreground window alone so active interaction stays smooth.
- **Interval (ms)** — How often the worker runs. Smaller = more aggressive trimming + more disk wear; larger = gentler. Range: 100 ms – 600 000 ms (10 minutes).
- **Triggers** — Comma-separated list of process names (e.g. `chrome, discord`). When non-empty, the limiter only acts while at least one of these processes is running. Leave empty for "always on".

### Soft limit mode

By default the limiter trims a process working set down to its minimum, asking Windows to swap most pages out. For some apps this is too aggressive and causes stuttering. Right-click a row → **Set soft limit…**, type a value in MB, and the limiter will instead cap the working set at that size — pages above the cap are evicted, pages below stay resident. **Clear soft limit** restores trim mode.

### Profiles

Click **Profiles…** to open the profile manager. A profile captures the full set of monitored apps plus the master/skip-focused/interval/triggers settings, all under a chosen name. Use cases:

- "Gaming" — only Chrome, Discord, Spotify enabled, 1.5 s interval, trigger on `<your-game>.exe`.
- "Office" — soft-limit Outlook to 800 MB, master enabled, skip focused.
- "Idle" — master enabled, no triggers, 10 s interval, trim everything.

Activate a profile to load it. Save the current settings as a new profile from the same view. Deletion just removes the saved snapshot — your current settings are unaffected.

### Live statistics

Below the app list:

- **Status line** — total RAM across monitored processes, how many are active, master-switch state.
- **Stats line** — `Trimmed: <N MB>` is an estimate of bytes the limiter pushed to the page file (sum of working-set drops at each trim cycle); `Page faults: <N>` is the cumulative page-fault count across all monitored processes.

### Autostart

Check "Start with Windows" to launch RAM Limiter automatically on login. The application will start minimized to the system tray.

### How It Works

RAM Limiter runs a background thread that:
1. Enumerates all running processes once per **trim interval** (configurable; defaults to 1.5 s).
2. For each enabled application:
   - If a **soft limit** is set, calls `SetProcessWorkingSetSizeEx(handle, 1 MiB, soft_limit, 0)` — the OS keeps the working set under the cap.
   - Otherwise calls `SetProcessWorkingSetSizeEx(handle, -1, -1, 0)` — Windows trims the working set to its minimum.
3. Windows moves the affected pages to the page file, freeing physical RAM.
4. The worker also reads `PROCESS_MEMORY_COUNTERS` to feed the page-fault and disk-write statistics in the UI.

The worker honors:
- The **master switch** (no work happens at all when off).
- **Triggers** (no work happens unless one of the configured trigger processes is running).
- **Skip focused** (the foreground process is left alone, even if it would otherwise be limited).

## 🛠️ Technical Details

### Architecture

- **Language:** Rust (2021 edition)
- **GUI Framework:** [native-windows-gui](https://github.com/gabdube/native-windows-gui)
- **Windows API:** Direct WinAPI calls via [winapi-rs](https://github.com/retep998/winapi-rs)

### Key Components

- `src/main.rs` — Entry point, handles `--minimized` flag
- `src/ui.rs` — GUI implementation (window, list view, dialogs)
- `src/limiter.rs` — Background worker thread, process enumeration, memory trimming
- `src/config.rs` — Configuration persistence, autostart registry management
- `src/icon_extract.rs` — Extracts icons from .exe files using `ExtractIconExW`

### Memory Trimming

The core technique uses the Windows API:
```rust
SetProcessWorkingSetSizeEx(handle, usize::MAX, usize::MAX, 0);
```

This is a legitimate Windows API call that instructs the OS to trim the process working set to its minimum size. While effective at freeing RAM, aggressive use can impact application performance.

## 🔒 Security & Privacy

- **No network access** — All operations are local
- **Minimal permissions** — Only requires `PROCESS_QUERY_INFORMATION` and `PROCESS_SET_QUOTA`

## 📝 Configuration

Settings are stored in JSON format at:
```
%APPDATA%\RamLimiter\config.json
```

Example:
```json
{
  "apps": {
    "C:\\Program Files\\App\\app.exe": {
      "exe_path": "C:\\Program Files\\App\\app.exe",
      "display_name": "My Application",
      "enabled": true,
      "soft_limit_mb": 500
    }
  },
  "autostart": false,
  "trim_interval_ms": 1500,
  "global_enabled": true,
  "skip_focused": true,
  "triggers": ["chrome", "discord"],
  "active_profile": "Gaming",
  "profiles": {
    "Gaming": {
      "apps": { "...": "..." },
      "trim_interval_ms": 1500,
      "global_enabled": true,
      "skip_focused": true,
      "triggers": ["yourgame"]
    },
    "Idle": {
      "apps": { "...": "..." },
      "trim_interval_ms": 10000,
      "global_enabled": true,
      "skip_focused": false,
      "triggers": []
    }
  }
}
```

All new top-level fields are optional and fall back to sensible defaults when missing, so configs from older versions continue to work.

## 🐛 Troubleshooting

**Application crashes after enabling limiter:**
- Disable limiting for that application
- Some apps don't handle aggressive memory trimming well
- Try limiting less frequently by modifying the sleep duration in `limiter.rs`

**"Access Denied" errors:**
- Some system processes are protected and cannot be trimmed
- Run RAM Limiter as Administrator (not recommended for security reasons)

**High disk usage:**
- This is expected behavior — trimmed memory is moved to the page file
- Reduce the number of limited applications or disable limiting for disk-intensive apps

**⚠️ WARNING: Disk Wear Concerns**

Aggressive memory trimming can significantly increase disk wear, especially on SSDs:

- **Write amplification:** Every 1.5 seconds, Windows may write hundreds of MB to the page file
- **Estimated wear:** 30-300 GB of writes per day per limited application (depending on usage patterns)
- **SSD lifespan impact:** Can reduce SSD lifespan by months or years with continuous use
- **HDD impact:** Increased mechanical wear, noise, and slower performance due to constant head movement

**Recommendations to minimize disk wear:**
- Only limit applications when RAM is critically low (< 20% free)
- Prefer limiting background/idle applications rather than actively used ones
- Consider increasing the trim interval in `limiter.rs` from 1.5s to 5-10s
- Monitor disk write statistics using tools like CrystalDiskInfo
- Avoid limiting applications on systems with sufficient RAM (16+ GB)

**Icons not showing:**
- Some executables don't contain embedded icons
- The app will still work, just without the visual icon

## 🚧 Roadmap

All previously-planned features have shipped:

- [x] **Custom trim interval** — Configure the trim frequency in milliseconds (toolbar input).
- [x] **Launch on specific app start** — Trigger processes gate the limiter; configurable from the toolbar.
- [x] **Global on/off switch** — "Master enabled" checkbox in the toolbar.
- [x] **Soft limit mode** — Per-app working-set cap (right-click → *Set soft limit…*).
- [x] **Skip focused windows** — "Skip focused" checkbox in the toolbar.
- [x] **Page fault monitoring** — Per-app counter column + total in the stats line.
- [x] **Disk write statistics** — Estimated bytes pushed to the page file in the stats line.
- [x] **Profiles / presets** — Profile manager with activate / save current / delete.

Contributions for further improvements are welcome — feel free to open an issue or pull request.

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Inspired by the [Discord RAM Limiter](https://github.com/farajyeet/discord-ram-limiter)
- Built with [native-windows-gui](https://github.com/gabdube/native-windows-gui)
- Uses the [winapi-rs](https://github.com/retep998/winapi-rs) crate for Windows API bindings

## 📧 Contact

For bugs, feature requests, or questions, please open an issue on GitHub.