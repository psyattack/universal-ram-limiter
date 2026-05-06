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
   - Click "+ Browse .exe…"
   - Select the application's .exe file
   - The app will be added to the list with its icon

2. **Pick from running processes:**
   - Click "Pick running process…"
   - Select a process from the list (sorted by RAM usage)
   - Click "Add Selected"

### Managing Applications

- **Enable/Disable:** Double-click an application in the list to toggle limiting
- **Remove:** Select an application and click "Remove Selected"
- **View RAM usage:** The "RAM Usage" column shows current memory consumption

### Autostart

Check "Start with Windows" to launch RAM Limiter automatically on login. The application will start minimized to the system tray.

### How It Works

RAM Limiter runs a background thread that:
1. Enumerates all running processes every 1.5 seconds
2. For each enabled application, calls `SetProcessWorkingSetSize(handle, -1, -1)`
3. This tells Windows to trim the working set to its minimum size
4. Windows moves unused memory pages to the page file, freeing physical RAM

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
      "enabled": true
    }
  },
  "autostart": false
}
```

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

## 🚧 TODO / Planned Features

Future improvements planned for RAM Limiter:

- [ ] **Custom trim interval** — Allow users to configure the trim frequency (e.g., 1s, 5s, 10s, 30s)
- [ ] **Launch on specific app start** — Automatically start limiting when certain applications are launched
- [ ] **Global on/off switch** — Master toggle to pause all limiting without disabling individual apps
- [ ] **Soft limit mode** — Set a specific memory limit (e.g., 500 MB) instead of trimming to minimum
- [ ] **Skip focused windows** — Don't trim applications that currently have focus to avoid stuttering during active use
- [ ] **Page fault monitoring** — Display page fault statistics to help users identify performance issues
- [ ] **Disk write statistics** — Show estimated disk writes caused by memory trimming
- [ ] **Profiles/presets** — Save and load different limiting configurations for different scenarios

Contributions are welcome! Feel free to open an issue or pull request for any of these features.

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Inspired by the [Discord RAM Limiter](https://github.com/farajyeet/discord-ram-limiter)
- Built with [native-windows-gui](https://github.com/gabdube/native-windows-gui)
- Uses the [winapi-rs](https://github.com/retep998/winapi-rs) crate for Windows API bindings

## 📧 Contact

For bugs, feature requests, or questions, please open an issue on GitHub.

---

**Remember:** This tool modifies system memory behavior. Always test with non-critical applications first and monitor system stability.
