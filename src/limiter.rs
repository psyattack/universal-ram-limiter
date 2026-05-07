//! Background limiter service.
//!
//! Periodically calls `SetProcessWorkingSetSizeEx` on every enabled target
//! process. By default, the call asks Windows to trim the working set to its
//! minimum (the original behavior). When a per-app `soft_limit_mb` is
//! configured, the call instead caps the working set at that size — Windows
//! still trims, but only down to roughly the requested ceiling.
//!
//! The worker also collects per-cycle telemetry — page-fault counters and an
//! estimated number of bytes pushed into the page file — which the UI reads
//! out of [`LimiterStats`] for the “page fault monitoring” and “disk write
//! statistics” features.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Per-target runtime info that the worker uses to decide what to do.
#[derive(Debug, Clone, Default)]
pub struct AppRuntime {
    /// User-facing display name (used as the key for per-app stats).
    pub display_name: String,
    /// Whether limiting is enabled for this app.
    pub enabled: bool,
    /// When set, the worker caps the working set at this many MB instead of
    /// trimming to the minimum.
    pub soft_limit_mb: Option<u64>,
}

/// Stats accumulated by the worker. The UI reads these directly to show
/// page-fault and disk-write information.
#[derive(Debug, Clone, Default)]
pub struct LimiterStats {
    /// Approximate cumulative bytes pushed to the page file by trimming.
    /// Computed as the sum of `pre_ws - post_ws` over every trimmed pid.
    pub estimated_pagefile_bytes: u64,
    /// Total page-fault count across every visible target process at the
    /// last cycle. This is the absolute counter from
    /// `PROCESS_MEMORY_COUNTERS::PageFaultCount`, not a delta.
    pub last_page_faults_total: u64,
    /// Total working-set bytes across every visible target process at the
    /// last cycle (after any trim).
    pub last_working_set_total: u64,
    /// Total number of target processes seen at the last cycle.
    pub last_process_count: u32,
    /// Per-app page-fault counters at the last cycle (display_name → count).
    pub app_page_faults: HashMap<String, u64>,
    /// Per-app working-set total at the last cycle (display_name → bytes).
    pub app_working_set: HashMap<String, u64>,
    /// Per-app process count at the last cycle.
    pub app_process_count: HashMap<String, u32>,
    /// Whether the most recent cycle actually applied trim/limit calls.
    /// Used by the UI to communicate "paused" state in the status line.
    pub last_cycle_applied: bool,
}

/// Shared state between the UI thread and the background worker.
#[derive(Debug)]
pub struct LimiterState {
    /// process_name (without .exe, lowercased) → runtime info.
    pub apps: HashMap<String, AppRuntime>,
    /// When false, the worker thread should exit.
    pub running: bool,
    /// Master on/off — when false, no trimming is applied even if individual
    /// apps are enabled.
    pub global_enabled: bool,
    /// When true, the foreground process is left alone to avoid stutter.
    pub skip_focused: bool,
    /// How long to sleep between trim cycles.
    pub trim_interval_ms: u64,
    /// Trigger process names (lowercased, without .exe). When non-empty,
    /// limiting only happens while at least one trigger process is running.
    pub triggers: HashSet<String>,
    /// Stats produced by the worker for the UI.
    pub stats: LimiterStats,
}

impl Default for LimiterState {
    fn default() -> Self {
        Self {
            apps: HashMap::new(),
            running: true,
            global_enabled: true,
            skip_focused: false,
            trim_interval_ms: 1500,
            triggers: HashSet::new(),
            stats: LimiterStats::default(),
        }
    }
}

/// Result of one enumeration cycle. Built up by [`scan_and_apply`] and
/// folded back into [`LimiterStats`] by [`merge_summary`].
#[derive(Debug, Default)]
struct CycleSummary {
    /// pid → working-set in bytes BEFORE trimming this cycle.
    pre_ws: HashMap<u32, u64>,
    /// pid → working-set in bytes AFTER trimming this cycle (or `pre_ws`
    /// when trimming was skipped because the process is in the foreground
    /// or master toggle is off).
    post_ws: HashMap<u32, u64>,
    /// pid → latest `PageFaultCount`.
    page_faults: HashMap<u32, u32>,
    /// pid → owning app `display_name` (so the UI can group per-app).
    pid_to_app: HashMap<u32, String>,
    /// Whether trim/limit calls were issued this cycle.
    applied_trim: bool,
}

/// Start the background worker. The optional `repaint` callback is invoked
/// after each cycle so the UI can refresh its displays without polling.
pub fn start_worker(state: Arc<Mutex<LimiterState>>, repaint: Option<Box<dyn Fn() + Send>>) {
    thread::spawn(move || loop {
        // Snapshot the bits the worker needs without holding the lock.
        let (running, global_enabled, skip_focused, sleep_ms, triggers, targets) = {
            let st = state.lock().unwrap();
            let targets: Vec<(String, AppRuntime)> = st
                .apps
                .iter()
                .filter(|(_, a)| a.enabled)
                .map(|(k, a)| (k.clone(), a.clone()))
                .collect();
            (
                st.running,
                st.global_enabled,
                st.skip_focused,
                st.trim_interval_ms.max(100),
                st.triggers.clone(),
                targets,
            )
        };

        if !running {
            break;
        }

        // Decide whether this cycle should actually trim. We always SCAN
        // (so the UI gets fresh stats), but only trim when permitted.
        let allow_trim = global_enabled
            && !targets.is_empty()
            && (triggers.is_empty() || triggers_running(&triggers));

        let foreground_pid = if skip_focused && allow_trim {
            Some(get_foreground_pid())
        } else {
            None
        };

        let summary = scan_and_apply(&targets, foreground_pid, allow_trim);
        {
            let mut st = state.lock().unwrap();
            merge_summary(&mut st.stats, &summary);
        }

        if let Some(ref cb) = repaint {
            cb();
        }

        thread::sleep(Duration::from_millis(sleep_ms));
    });
}

/// Fold a cycle summary into the persistent stats.
fn merge_summary(stats: &mut LimiterStats, sum: &CycleSummary) {
    // Disk-write estimate: sum of positive `pre - post` deltas.
    let mut shed: u64 = 0;
    for (pid, &pre) in &sum.pre_ws {
        if let Some(&post) = sum.post_ws.get(pid) {
            if pre > post {
                shed = shed.saturating_add(pre - post);
            }
        }
    }
    stats.estimated_pagefile_bytes = stats.estimated_pagefile_bytes.saturating_add(shed);

    // Per-app aggregation (latest snapshot, not cumulative).
    let mut app_pf: HashMap<String, u64> = HashMap::new();
    let mut app_ws: HashMap<String, u64> = HashMap::new();
    let mut app_pc: HashMap<String, u32> = HashMap::new();

    for (pid, app) in &sum.pid_to_app {
        if let Some(&ws) = sum.post_ws.get(pid) {
            *app_ws.entry(app.clone()).or_insert(0) += ws;
        }
        if let Some(&pf) = sum.page_faults.get(pid) {
            *app_pf.entry(app.clone()).or_insert(0) += pf as u64;
        }
        *app_pc.entry(app.clone()).or_insert(0) += 1;
    }

    stats.last_page_faults_total = app_pf.values().sum();
    stats.last_working_set_total = app_ws.values().sum();
    stats.last_process_count = app_pc.values().sum();
    stats.app_page_faults = app_pf;
    stats.app_working_set = app_ws;
    stats.app_process_count = app_pc;
    stats.last_cycle_applied = sum.applied_trim;
}

// ---------------------------------------------------------------------------
// Windows implementation: enumeration + trim + scan
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn scan_and_apply(
    targets: &[(String, AppRuntime)],
    foreground_pid: Option<u32>,
    allow_trim: bool,
) -> CycleSummary {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::memoryapi::SetProcessWorkingSetSizeEx;
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::psapi::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use winapi::um::tlhelp32::*;
    use winapi::um::winnt::{PROCESS_QUERY_INFORMATION, PROCESS_SET_QUOTA};

    let mut summary = CycleSummary::default();
    if targets.is_empty() {
        return summary;
    }

    // Index targets by lowercased process name (without .exe).
    let mut idx: HashMap<&str, &AppRuntime> = HashMap::new();
    for (name, rt) in targets {
        idx.insert(name.as_str(), rt);
    }

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap == winapi::um::handleapi::INVALID_HANDLE_VALUE {
            return summary;
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snap, &mut entry) == 0 {
            CloseHandle(snap);
            return summary;
        }

        loop {
            let exe_name = {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                OsString::from_wide(&entry.szExeFile[..len])
                    .to_string_lossy()
                    .to_lowercase()
            };
            let base = exe_name
                .strip_suffix(".exe")
                .unwrap_or(&exe_name)
                .to_string();

            let target_match = idx
                .get(base.as_str())
                .copied()
                .or_else(|| idx.get(exe_name.as_str()).copied());

            if let Some(rt) = target_match {
                let pid = entry.th32ProcessID;

                let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA, 0, pid);
                if !handle.is_null() {
                    // Read pre-trim memory info.
                    let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
                    pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
                    let pre_ok = GetProcessMemoryInfo(
                        handle,
                        &mut pmc,
                        std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                    ) != 0;
                    let pre_ws = if pre_ok { pmc.WorkingSetSize as u64 } else { 0 };
                    let page_faults = if pre_ok { pmc.PageFaultCount } else { 0 };

                    // Decide whether to trim this pid.
                    let is_foreground = foreground_pid.map_or(false, |fp| fp != 0 && fp == pid);
                    let mut post_ws = pre_ws;

                    if allow_trim && !is_foreground {
                        match rt.soft_limit_mb {
                            Some(mb) => {
                                let max_bytes = mb.saturating_mul(1024 * 1024).max(2 * 1024 * 1024);
                                let min_bytes: usize = 1024 * 1024;
                                SetProcessWorkingSetSizeEx(
                                    handle,
                                    min_bytes,
                                    max_bytes as usize,
                                    0,
                                );
                            }
                            None => {
                                // -1, -1 → tell Windows to trim to minimum.
                                SetProcessWorkingSetSizeEx(handle, usize::MAX, usize::MAX, 0);
                            }
                        }
                        summary.applied_trim = true;

                        // Re-read working-set so we can count what was shed.
                        let mut pmc2: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
                        pmc2.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
                        if GetProcessMemoryInfo(
                            handle,
                            &mut pmc2,
                            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                        ) != 0
                        {
                            post_ws = pmc2.WorkingSetSize as u64;
                        }
                    }

                    summary.pre_ws.insert(pid, pre_ws);
                    summary.post_ws.insert(pid, post_ws);
                    summary.page_faults.insert(pid, page_faults);
                    summary.pid_to_app.insert(pid, rt.display_name.clone());

                    CloseHandle(handle);
                }
            }

            if Process32NextW(snap, &mut entry) == 0 {
                break;
            }
        }
        CloseHandle(snap);
    }

    summary
}

#[cfg(not(target_os = "windows"))]
fn scan_and_apply(
    _targets: &[(String, AppRuntime)],
    _foreground_pid: Option<u32>,
    _allow_trim: bool,
) -> CycleSummary {
    CycleSummary::default()
}

#[cfg(target_os = "windows")]
fn get_foreground_pid() -> u32 {
    use winapi::um::winuser::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return 0;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        pid
    }
}

#[cfg(not(target_os = "windows"))]
fn get_foreground_pid() -> u32 {
    0
}

/// Returns true when at least one of the given process names is currently
/// running. Names are matched case-insensitively, with or without a `.exe`
/// suffix.
#[cfg(target_os = "windows")]
fn triggers_running(triggers: &HashSet<String>) -> bool {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::tlhelp32::*;

    if triggers.is_empty() {
        return true;
    }

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap == winapi::um::handleapi::INVALID_HANDLE_VALUE {
            return false;
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snap, &mut entry) == 0 {
            CloseHandle(snap);
            return false;
        }

        loop {
            let exe_name = {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                OsString::from_wide(&entry.szExeFile[..len])
                    .to_string_lossy()
                    .to_lowercase()
            };
            let base = exe_name
                .strip_suffix(".exe")
                .unwrap_or(&exe_name)
                .to_string();
            if triggers.contains(&base) || triggers.contains(&exe_name) {
                CloseHandle(snap);
                return true;
            }
            if Process32NextW(snap, &mut entry) == 0 {
                break;
            }
        }
        CloseHandle(snap);
    }

    false
}

#[cfg(not(target_os = "windows"))]
fn triggers_running(_triggers: &HashSet<String>) -> bool {
    false
}

// ---------------------------------------------------------------------------
// Helper: enumerate running processes (for the picker view).
// ---------------------------------------------------------------------------

/// A single running-process snapshot used by the “pick from running
/// processes” dialog.
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    /// Executable file name (e.g. `chrome.exe`).
    pub name: String,
    /// Full path to the executable on disk. May be empty if the OS denied
    /// access.
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

            // Skip the System Idle Process and System (PIDs 0/4) — they
            // cannot be trimmed.
            if pid == 0 || pid == 4 || exe_name.is_empty() {
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
                continue;
            }

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

    // Heaviest first.
    out.sort_by_key(|p| std::cmp::Reverse(p.working_set));
    out
}

#[cfg(not(target_os = "windows"))]
pub fn enumerate_processes() -> Vec<ProcessInfo> {
    Vec::new()
}
