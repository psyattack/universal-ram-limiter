//! Background limiter service.
//!
//! Periodically calls `SetProcessWorkingSetSize(handle, -1, -1)` on every
//! enabled target process, which asks the OS to trim the working set.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Shared state between the UI thread and the background worker.
#[derive(Debug)]
pub struct LimiterState {
    /// Process names (without .exe) that should be trimmed.
    pub targets: HashSet<String>,
    /// When false the worker thread should exit.
    pub running: bool,
}

impl Default for LimiterState {
    fn default() -> Self {
        Self {
            targets: HashSet::new(),
            running: true,
        }
    }
}

/// Start the background worker. The optional `repaint` callback is invoked
/// after each trim cycle so the UI can refresh its RAM read-out without
/// having to poll the OS on its own clock.
pub fn start_worker(state: Arc<Mutex<LimiterState>>, repaint: Option<Box<dyn Fn() + Send>>) {
    thread::spawn(move || loop {
        {
            let st = state.lock().unwrap();
            if !st.running {
                break;
            }
            let names: Vec<String> = st.targets.iter().cloned().collect();
            drop(st);

            if !names.is_empty() {
                trim_processes(&names);
            }
        }

        if let Some(ref cb) = repaint {
            cb();
        }

        thread::sleep(Duration::from_millis(1500));
    });
}

// ---------------------------------------------------------------------------
// Windows implementation
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn trim_processes(names: &[String]) {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::memoryapi::SetProcessWorkingSetSizeEx;
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::tlhelp32::*;
    use winapi::um::winnt::{PROCESS_QUERY_INFORMATION, PROCESS_SET_QUOTA};

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap == winapi::um::handleapi::INVALID_HANDLE_VALUE {
            return;
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snap, &mut entry) == 0 {
            CloseHandle(snap);
            return;
        }

        loop {
            let exe_name = {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let os = OsString::from_wide(&entry.szExeFile[..len]);
                os.to_string_lossy().to_string()
            };

            // Strip .exe suffix for comparison.
            let base = exe_name
                .strip_suffix(".exe")
                .or_else(|| exe_name.strip_suffix(".EXE"))
                .unwrap_or(&exe_name);

            let matches = names
                .iter()
                .any(|n| n.eq_ignore_ascii_case(base) || n.eq_ignore_ascii_case(&exe_name));
            if matches {
                let handle = OpenProcess(
                    PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA,
                    0,
                    entry.th32ProcessID,
                );
                if !handle.is_null() {
                    // -1 tells Windows to trim the working set to its minimum.
                    SetProcessWorkingSetSizeEx(handle, usize::MAX, usize::MAX, 0);
                    CloseHandle(handle);
                }
            }

            if Process32NextW(snap, &mut entry) == 0 {
                break;
            }
        }
        CloseHandle(snap);
    }
}

#[cfg(not(target_os = "windows"))]
fn trim_processes(_names: &[String]) {
    // No-op on non-Windows — the binary only runs on Windows.
}

// ---------------------------------------------------------------------------
// Helper: enumerate running processes (for RAM display).
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub fn get_working_set_bytes(process_names: &[String]) -> (u64, u32) {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::psapi::GetProcessMemoryInfo;
    use winapi::um::psapi::PROCESS_MEMORY_COUNTERS;
    use winapi::um::tlhelp32::*;
    use winapi::um::winnt::PROCESS_QUERY_INFORMATION;

    let mut total_bytes: u64 = 0;
    let mut count: u32 = 0;

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap == winapi::um::handleapi::INVALID_HANDLE_VALUE {
            return (0, 0);
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snap, &mut entry) == 0 {
            CloseHandle(snap);
            return (0, 0);
        }

        loop {
            let exe_name = {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let os = OsString::from_wide(&entry.szExeFile[..len]);
                os.to_string_lossy().to_string()
            };

            let base = exe_name
                .strip_suffix(".exe")
                .or_else(|| exe_name.strip_suffix(".EXE"))
                .unwrap_or(&exe_name);

            let matches = process_names
                .iter()
                .any(|n| n.eq_ignore_ascii_case(base) || n.eq_ignore_ascii_case(&exe_name));

            if matches {
                let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, entry.th32ProcessID);
                if !handle.is_null() {
                    let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
                    pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
                    if GetProcessMemoryInfo(
                        handle,
                        &mut pmc,
                        std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                    ) != 0
                    {
                        total_bytes += pmc.WorkingSetSize as u64;
                        count += 1;
                    }
                    CloseHandle(handle);
                }
            }

            if Process32NextW(snap, &mut entry) == 0 {
                break;
            }
        }
        CloseHandle(snap);
    }

    (total_bytes, count)
}

#[cfg(not(target_os = "windows"))]
pub fn get_working_set_bytes(_process_names: &[String]) -> (u64, u32) {
    (0, 0)
}

// ---------------------------------------------------------------------------
// Helper: enumerate all running processes with PID, name, full path, RAM.
// Used by the "pick from running processes" dialog.
// ---------------------------------------------------------------------------

/// A single running process snapshot.
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    /// Executable file name (e.g. "chrome.exe").
    pub name: String,
    /// Full path to the executable on disk. May be empty if the OS denied access.
    pub exe_path: String,
    /// Working-set size in bytes at enumeration time.
    pub working_set: u64,
}

#[cfg(target_os = "windows")]
pub fn enumerate_processes() -> Vec<ProcessInfo> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use winapi::shared::minwindef::DWORD;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::psapi::GetProcessMemoryInfo;
    use winapi::um::psapi::PROCESS_MEMORY_COUNTERS;
    use winapi::um::tlhelp32::*;
    use winapi::um::winbase::QueryFullProcessImageNameW;
    use winapi::um::winnt::{
        PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    };

    let mut out: Vec<ProcessInfo> = Vec::with_capacity(256);

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap == winapi::um::handleapi::INVALID_HANDLE_VALUE {
            return out;
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snap, &mut entry) == 0 {
            CloseHandle(snap);
            return out;
        }

        loop {
            let exe_name = {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let os = OsString::from_wide(&entry.szExeFile[..len]);
                os.to_string_lossy().to_string()
            };

            let pid = entry.th32ProcessID;

            // Skip the System Idle Process and System (PIDs 0/4) — they cannot be trimmed.
            if pid == 0 || pid == 4 || exe_name.is_empty() {
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
                continue;
            }

            // Try to fetch the full path + working-set size. Some processes (e.g.
            // protected/system) will refuse PROCESS_QUERY_INFORMATION; fall back to
            // PROCESS_QUERY_LIMITED_INFORMATION for the path.
            let mut full_path = String::new();
            let mut working_set: u64 = 0;

            let h = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
            if !h.is_null() {
                let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
                pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
                if GetProcessMemoryInfo(
                    h,
                    &mut pmc,
                    std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                ) != 0
                {
                    working_set = pmc.WorkingSetSize as u64;
                }

                let mut buf = [0u16; 1024];
                let mut size: DWORD = buf.len() as DWORD;
                if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut size) != 0 {
                    full_path = OsString::from_wide(&buf[..size as usize])
                        .to_string_lossy()
                        .into_owned();
                }
                CloseHandle(h);
            }

            if full_path.is_empty() {
                let h2 = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if !h2.is_null() {
                    let mut buf = [0u16; 1024];
                    let mut size: DWORD = buf.len() as DWORD;
                    if QueryFullProcessImageNameW(h2, 0, buf.as_mut_ptr(), &mut size) != 0 {
                        full_path = OsString::from_wide(&buf[..size as usize])
                            .to_string_lossy()
                            .into_owned();
                    }
                    CloseHandle(h2);
                }
            }

            out.push(ProcessInfo {
                pid,
                name: exe_name,
                exe_path: full_path,
                working_set,
            });

            if Process32NextW(snap, &mut entry) == 0 {
                break;
            }
        }
        CloseHandle(snap);
    }

    // Sort by working-set size descending so heavy hitters surface first.
    out.sort_by_key(|p| std::cmp::Reverse(p.working_set));
    out
}

#[cfg(not(target_os = "windows"))]
pub fn enumerate_processes() -> Vec<ProcessInfo> {
    Vec::new()
}
