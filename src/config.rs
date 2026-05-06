use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Per-application entry persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppEntry {
    /// Full path to the executable.
    pub exe_path: String,
    /// Display name shown in the list.
    pub display_name: String,
    /// Whether the limiter is active for this app.
    pub enabled: bool,
}

/// Top-level config stored as JSON.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Map from exe_path → AppEntry.
    pub apps: HashMap<String, AppEntry>,
    /// Whether to start on Windows login.
    pub autostart: bool,
}

impl Config {
    /// Returns the path to the config file in %APPDATA%/RamLimiter/config.json
    pub fn path() -> PathBuf {
        let base = dirs_next::config_dir().unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join("RamLimiter");
        let _ = fs::create_dir_all(&dir);
        dir.join("config.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        match fs::read_to_string(&path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, data);
        }
    }
}

// ---------------------------------------------------------------------------
// Windows autostart (registry Run key)
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub mod autostart {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use winapi::shared::minwindef::*;
    use winapi::shared::winerror::ERROR_SUCCESS;
    use winapi::um::winnt::{KEY_READ, KEY_WRITE};
    use winapi::um::winreg::*;

    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const VALUE_NAME: &str = "RamLimiter";

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    #[allow(dead_code)]
    pub fn is_enabled() -> bool {
        unsafe {
            let mut hkey: HKEY = std::ptr::null_mut();
            let key = to_wide(RUN_KEY);
            if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_READ, &mut hkey)
                != ERROR_SUCCESS as i32
            {
                return false;
            }
            let name = to_wide(VALUE_NAME);
            let result = RegQueryValueExW(
                hkey,
                name.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            RegCloseKey(hkey);
            result == ERROR_SUCCESS as i32
        }
    }

    pub fn set_enabled(enable: bool) {
        unsafe {
            let mut hkey: HKEY = std::ptr::null_mut();
            let key = to_wide(RUN_KEY);
            if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_WRITE, &mut hkey)
                != ERROR_SUCCESS as i32
            {
                return;
            }

            let name = to_wide(VALUE_NAME);
            if enable {
                if let Ok(exe) = std::env::current_exe() {
                    let cmd = format!("\"{}\" --minimized", exe.display());
                    let wide_cmd = to_wide(&cmd);
                    RegSetValueExW(
                        hkey,
                        name.as_ptr(),
                        0,
                        winapi::um::winnt::REG_SZ,
                        wide_cmd.as_ptr() as *const u8,
                        (wide_cmd.len() * 2) as u32,
                    );
                }
            } else {
                RegDeleteValueW(hkey, name.as_ptr());
            }
            RegCloseKey(hkey);
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub mod autostart {
    #[allow(dead_code)]
    pub fn is_enabled() -> bool {
        false
    }
    pub fn set_enabled(_enable: bool) {}
}
