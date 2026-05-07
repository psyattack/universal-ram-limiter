//! Native Windows GUI for the universal RAM Limiter.
//!
//! The window has four interchangeable views, all hosted in the same
//! top-level window (we hide one set of controls and show another):
//!
//! * **Main**     — toolbar, settings row, monitored-apps list, status/stats.
//! * **Picker**   — pick from running processes (filterable).
//! * **Profiles** — manage saved profiles (activate / delete / save current).
//! * **SoftLimit**— edit the per-app soft working-set cap.

use crate::config::{autostart, AppEntry, Config};
use crate::icon_extract;
use crate::limiter::{self, AppRuntime, LimiterState, ProcessInfo};
use native_windows_gui as nwg;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

#[cfg(target_os = "windows")]
fn show_hwnd(handle: &nwg::ControlHandle, show: bool) {
    use winapi::um::winuser::{ShowWindow, SW_HIDE, SW_SHOW};
    if let Some(hwnd) = handle.hwnd() {
        unsafe {
            ShowWindow(hwnd, if show { SW_SHOW } else { SW_HIDE });
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn show_hwnd(_handle: &nwg::ControlHandle, _show: bool) {}

#[cfg(target_os = "windows")]
fn focus_hwnd(handle: &nwg::ControlHandle) {
    use winapi::um::winuser::SetForegroundWindow;
    if let Some(hwnd) = handle.hwnd() {
        unsafe {
            SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn focus_hwnd(_handle: &nwg::ControlHandle) {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Main,
    Picker,
    Profiles,
    SoftLimit,
}

/// Runtime state for a single row in the apps list.
struct AppRow {
    exe_path: String,
    display_name: String,
    enabled: bool,
    soft_limit_mb: Option<u64>,
    #[allow(dead_code)]
    icon_path: Option<PathBuf>,
}

struct AppState {
    config: Config,
    rows: Vec<AppRow>,
    limiter: Arc<Mutex<LimiterState>>,
    /// Index of the row currently being edited in the soft-limit view.
    editing_idx: Option<usize>,
}

pub fn run(start_minimized: bool) {
    nwg::init().expect("Failed to init NWG");
    nwg::Font::set_global_family("Segoe UI").expect("Failed to set font");

    let config = Config::load();

    // ── Window ──────────────────────────────────────────────────────────
    let mut window = nwg::Window::default();
    nwg::Window::builder()
        .size((580, 540))
        .position((300, 150))
        .title("RAM Limiter")
        .flags(
            nwg::WindowFlags::WINDOW | nwg::WindowFlags::VISIBLE | nwg::WindowFlags::MINIMIZE_BOX,
        )
        .build(&mut window)
        .expect("Window");

    // ── Toolbar row 1 ───────────────────────────────────────────────────
    let mut browse_btn = nwg::Button::default();
    nwg::Button::builder()
        .text("Browse .exe…")
        .size((130, 28))
        .position((12, 10))
        .parent(&window)
        .build(&mut browse_btn)
        .expect("browse btn");

    let mut pick_btn = nwg::Button::default();
    nwg::Button::builder()
        .text("Pick running process…")
        .size((160, 28))
        .position((146, 10))
        .parent(&window)
        .build(&mut pick_btn)
        .expect("pick btn");

    let mut profiles_btn = nwg::Button::default();
    nwg::Button::builder()
        .text("Profiles…")
        .size((90, 28))
        .position((310, 10))
        .parent(&window)
        .build(&mut profiles_btn)
        .expect("profiles btn");

    let mut master_cb = nwg::CheckBox::default();
    nwg::CheckBox::builder()
        .text("Master enabled")
        .size((150, 28))
        .position((404, 10))
        .parent(&window)
        .check_state(if config.global_enabled {
            nwg::CheckBoxState::Checked
        } else {
            nwg::CheckBoxState::Unchecked
        })
        .build(&mut master_cb)
        .expect("master cb");

    // ── Toolbar row 2 ───────────────────────────────────────────────────
    let mut skip_focus_cb = nwg::CheckBox::default();
    nwg::CheckBox::builder()
        .text("Skip focused")
        .size((130, 24))
        .position((12, 46))
        .parent(&window)
        .check_state(if config.skip_focused {
            nwg::CheckBoxState::Checked
        } else {
            nwg::CheckBoxState::Unchecked
        })
        .build(&mut skip_focus_cb)
        .expect("skip focused cb");

    let mut interval_label = nwg::Label::default();
    nwg::Label::builder()
        .text("Interval (ms):")
        .size((90, 24))
        .position((146, 48))
        .parent(&window)
        .build(&mut interval_label)
        .expect("interval label");

    let mut interval_input = nwg::TextInput::default();
    nwg::TextInput::builder()
        .text(&config.trim_interval_ms.to_string())
        .size((60, 24))
        .position((238, 46))
        .parent(&window)
        .build(&mut interval_input)
        .expect("interval input");

    let mut triggers_label = nwg::Label::default();
    nwg::Label::builder()
        .text("Triggers:")
        .size((60, 24))
        .position((306, 48))
        .parent(&window)
        .build(&mut triggers_label)
        .expect("triggers label");

    let mut triggers_input = nwg::TextInput::default();
    nwg::TextInput::builder()
        .text(&config.triggers.join(", "))
        .placeholder_text(Some("e.g. chrome, discord (empty = always on)"))
        .size((192, 24))
        .position((370, 46))
        .parent(&window)
        .build(&mut triggers_input)
        .expect("triggers input");

    // ── Main apps list ──────────────────────────────────────────────────
    let mut list_view = nwg::ListView::default();
    nwg::ListView::builder()
        .parent(&window)
        .size((556, 300))
        .position((12, 78))
        .list_style(nwg::ListViewStyle::Detailed)
        .flags(nwg::ListViewFlags::VISIBLE | nwg::ListViewFlags::SINGLE_SELECTION)
        .ex_flags(nwg::ListViewExFlags::FULL_ROW_SELECT | nwg::ListViewExFlags::GRID)
        .build(&mut list_view)
        .expect("list view");
    list_view.set_headers_enabled(true);
    list_view.insert_column(nwg::InsertListViewColumn {
        index: Some(0),
        fmt: None,
        width: Some(200),
        text: Some("Application".to_string()),
    });
    list_view.insert_column(nwg::InsertListViewColumn {
        index: Some(1),
        fmt: None,
        width: Some(60),
        text: Some("Status".to_string()),
    });
    list_view.insert_column(nwg::InsertListViewColumn {
        index: Some(2),
        fmt: None,
        width: Some(80),
        text: Some("Soft cap".to_string()),
    });
    list_view.insert_column(nwg::InsertListViewColumn {
        index: Some(3),
        fmt: None,
        width: Some(80),
        text: Some("RAM".to_string()),
    });
    list_view.insert_column(nwg::InsertListViewColumn {
        index: Some(4),
        fmt: None,
        width: Some(120),
        text: Some("Page faults".to_string()),
    });

    let mut image_list = nwg::ImageList::default();
    nwg::ImageList::builder()
        .size((20, 20))
        .initial(8)
        .grow(4)
        .build(&mut image_list)
        .expect("image list");

    // ── Picker controls (start hidden) ──────────────────────────────────
    let mut picker_back = nwg::Button::default();
    nwg::Button::builder()
        .text("← Back")
        .size((80, 28))
        .position((12, 10))
        .parent(&window)
        .build(&mut picker_back)
        .expect("picker back");

    let mut picker_refresh = nwg::Button::default();
    nwg::Button::builder()
        .text("Refresh")
        .size((80, 28))
        .position((96, 10))
        .parent(&window)
        .build(&mut picker_refresh)
        .expect("picker refresh");

    let mut picker_add = nwg::Button::default();
    nwg::Button::builder()
        .text("Add selected")
        .size((110, 28))
        .position((180, 10))
        .parent(&window)
        .build(&mut picker_add)
        .expect("picker add");

    let mut picker_filter = nwg::TextInput::default();
    nwg::TextInput::builder()
        .text("")
        .placeholder_text(Some("Filter by name or PID…"))
        .size((266, 28))
        .position((294, 10))
        .parent(&window)
        .build(&mut picker_filter)
        .expect("picker filter");

    let mut picker_list = nwg::ListView::default();
    nwg::ListView::builder()
        .parent(&window)
        .size((556, 440))
        .position((12, 46))
        .list_style(nwg::ListViewStyle::Detailed)
        .flags(nwg::ListViewFlags::VISIBLE | nwg::ListViewFlags::SINGLE_SELECTION)
        .ex_flags(nwg::ListViewExFlags::FULL_ROW_SELECT | nwg::ListViewExFlags::GRID)
        .build(&mut picker_list)
        .expect("picker list");
    picker_list.set_headers_enabled(true);
    picker_list.insert_column(nwg::InsertListViewColumn {
        index: Some(0),
        fmt: None,
        width: Some(180),
        text: Some("Process".to_string()),
    });
    picker_list.insert_column(nwg::InsertListViewColumn {
        index: Some(1),
        fmt: None,
        width: Some(70),
        text: Some("PID".to_string()),
    });
    picker_list.insert_column(nwg::InsertListViewColumn {
        index: Some(2),
        fmt: None,
        width: Some(80),
        text: Some("RAM".to_string()),
    });
    picker_list.insert_column(nwg::InsertListViewColumn {
        index: Some(3),
        fmt: None,
        width: Some(220),
        text: Some("Path".to_string()),
    });

    show_hwnd(&picker_back.handle, false);
    show_hwnd(&picker_refresh.handle, false);
    show_hwnd(&picker_add.handle, false);
    show_hwnd(&picker_filter.handle, false);
    show_hwnd(&picker_list.handle, false);

    // ── Profiles view controls ──────────────────────────────────────────
    let mut profile_back = nwg::Button::default();
    nwg::Button::builder()
        .text("← Back")
        .size((80, 28))
        .position((12, 10))
        .parent(&window)
        .build(&mut profile_back)
        .expect("profile back");

    let mut profile_header = nwg::Label::default();
    nwg::Label::builder()
        .text("Saved profiles — pick one and click Activate or Delete:")
        .size((556, 22))
        .position((12, 50))
        .parent(&window)
        .build(&mut profile_header)
        .expect("profile header");

    let mut profile_list = nwg::ListView::default();
    nwg::ListView::builder()
        .parent(&window)
        .size((360, 250))
        .position((12, 78))
        .list_style(nwg::ListViewStyle::Detailed)
        .flags(nwg::ListViewFlags::VISIBLE | nwg::ListViewFlags::SINGLE_SELECTION)
        .ex_flags(nwg::ListViewExFlags::FULL_ROW_SELECT)
        .build(&mut profile_list)
        .expect("profile list");
    profile_list.set_headers_enabled(true);
    profile_list.insert_column(nwg::InsertListViewColumn {
        index: Some(0),
        fmt: None,
        width: Some(220),
        text: Some("Profile".to_string()),
    });
    profile_list.insert_column(nwg::InsertListViewColumn {
        index: Some(1),
        fmt: None,
        width: Some(60),
        text: Some("Apps".to_string()),
    });
    profile_list.insert_column(nwg::InsertListViewColumn {
        index: Some(2),
        fmt: None,
        width: Some(60),
        text: Some("ms".to_string()),
    });

    let mut profile_activate = nwg::Button::default();
    nwg::Button::builder()
        .text("Activate")
        .size((110, 28))
        .position((380, 78))
        .parent(&window)
        .build(&mut profile_activate)
        .expect("profile activate");

    let mut profile_delete = nwg::Button::default();
    nwg::Button::builder()
        .text("Delete")
        .size((110, 28))
        .position((380, 114))
        .parent(&window)
        .build(&mut profile_delete)
        .expect("profile delete");

    let mut profile_active_label = nwg::Label::default();
    nwg::Label::builder()
        .text(&format!(
            "Active: {}",
            config.active_profile.as_deref().unwrap_or("(none)")
        ))
        .size((180, 22))
        .position((380, 156))
        .parent(&window)
        .build(&mut profile_active_label)
        .expect("profile active label");

    let mut profile_save_label = nwg::Label::default();
    nwg::Label::builder()
        .text("Save current settings as new profile:")
        .size((400, 22))
        .position((12, 348))
        .parent(&window)
        .build(&mut profile_save_label)
        .expect("profile save label");

    let mut profile_name_input = nwg::TextInput::default();
    nwg::TextInput::builder()
        .text("")
        .placeholder_text(Some("Profile name…"))
        .size((280, 28))
        .position((12, 376))
        .parent(&window)
        .build(&mut profile_name_input)
        .expect("profile name input");

    let mut profile_save_btn = nwg::Button::default();
    nwg::Button::builder()
        .text("Save current as…")
        .size((150, 28))
        .position((300, 376))
        .parent(&window)
        .build(&mut profile_save_btn)
        .expect("profile save btn");

    let mut profile_hint = nwg::Label::default();
    nwg::Label::builder()
        .text("A profile captures every monitored app, the trim interval, master/skip-focused/triggers settings.")
        .size((556, 44))
        .position((12, 416))
        .parent(&window)
        .build(&mut profile_hint)
        .expect("profile hint");

    show_hwnd(&profile_back.handle, false);
    show_hwnd(&profile_header.handle, false);
    show_hwnd(&profile_list.handle, false);
    show_hwnd(&profile_activate.handle, false);
    show_hwnd(&profile_delete.handle, false);
    show_hwnd(&profile_active_label.handle, false);
    show_hwnd(&profile_save_label.handle, false);
    show_hwnd(&profile_name_input.handle, false);
    show_hwnd(&profile_save_btn.handle, false);
    show_hwnd(&profile_hint.handle, false);

    // ── Soft-limit edit view controls ───────────────────────────────────
    let mut soft_back = nwg::Button::default();
    nwg::Button::builder()
        .text("← Back")
        .size((80, 28))
        .position((12, 10))
        .parent(&window)
        .build(&mut soft_back)
        .expect("soft back");

    let mut soft_title = nwg::Label::default();
    nwg::Label::builder()
        .text("Soft working-set cap")
        .size((556, 22))
        .position((12, 56))
        .parent(&window)
        .build(&mut soft_title)
        .expect("soft title");

    let mut soft_caption = nwg::Label::default();
    nwg::Label::builder()
        .text("Cap (MB) — leave empty to trim to minimum:")
        .size((556, 22))
        .position((12, 88))
        .parent(&window)
        .build(&mut soft_caption)
        .expect("soft caption");

    let mut soft_input = nwg::TextInput::default();
    nwg::TextInput::builder()
        .text("")
        .placeholder_text(Some("e.g. 500"))
        .size((100, 28))
        .position((12, 116))
        .parent(&window)
        .build(&mut soft_input)
        .expect("soft input");

    let mut soft_save = nwg::Button::default();
    nwg::Button::builder()
        .text("Save")
        .size((80, 28))
        .position((124, 116))
        .parent(&window)
        .build(&mut soft_save)
        .expect("soft save");

    let mut soft_clear = nwg::Button::default();
    nwg::Button::builder()
        .text("Clear")
        .size((80, 28))
        .position((212, 116))
        .parent(&window)
        .build(&mut soft_clear)
        .expect("soft clear");

    let mut soft_hint = nwg::Label::default();
    nwg::Label::builder()
        .text("In soft-limit mode the limiter caps the working set at the requested size instead of trimming to the minimum, which is gentler on apps that suffer from aggressive trimming.")
        .size((556, 60))
        .position((12, 156))
        .parent(&window)
        .build(&mut soft_hint)
        .expect("soft hint");

    show_hwnd(&soft_back.handle, false);
    show_hwnd(&soft_title.handle, false);
    show_hwnd(&soft_caption.handle, false);
    show_hwnd(&soft_input.handle, false);
    show_hwnd(&soft_save.handle, false);
    show_hwnd(&soft_clear.handle, false);
    show_hwnd(&soft_hint.handle, false);

    // ── Status / footer ─────────────────────────────────────────────────
    let mut status_label = nwg::Label::default();
    nwg::Label::builder()
        .text("No applications added")
        .size((556, 22))
        .position((12, 386))
        .parent(&window)
        .build(&mut status_label)
        .expect("status label");

    let mut stats_label = nwg::Label::default();
    nwg::Label::builder()
        .text("Trimmed: 0 MB · Page faults: 0")
        .size((556, 22))
        .position((12, 410))
        .parent(&window)
        .build(&mut stats_label)
        .expect("stats label");

    let mut autostart_cb = nwg::CheckBox::default();
    nwg::CheckBox::builder()
        .text("Launch at startup")
        .size((180, 24))
        .position((12, 438))
        .parent(&window)
        .check_state(if config.autostart {
            nwg::CheckBoxState::Checked
        } else {
            nwg::CheckBoxState::Unchecked
        })
        .build(&mut autostart_cb)
        .expect("autostart cb");

    let mut hint_label = nwg::Label::default();
    nwg::Label::builder()
        .text("Double-click row = toggle ON/OFF  •  Right-click = options")
        .size((556, 22))
        .position((12, 466))
        .parent(&window)
        .build(&mut hint_label)
        .expect("hint label");

    // ── Right-click context menu on the apps list ───────────────────────
    let mut list_menu = nwg::Menu::default();
    nwg::Menu::builder()
        .popup(true)
        .parent(&window)
        .build(&mut list_menu)
        .expect("list menu");

    let mut menu_toggle = nwg::MenuItem::default();
    nwg::MenuItem::builder()
        .text("Toggle ON/OFF")
        .parent(&list_menu)
        .build(&mut menu_toggle)
        .expect("menu toggle");

    let mut menu_set_soft = nwg::MenuItem::default();
    nwg::MenuItem::builder()
        .text("Set soft limit…")
        .parent(&list_menu)
        .build(&mut menu_set_soft)
        .expect("menu set soft");

    let mut menu_clear_soft = nwg::MenuItem::default();
    nwg::MenuItem::builder()
        .text("Clear soft limit")
        .parent(&list_menu)
        .build(&mut menu_clear_soft)
        .expect("menu clear soft");

    let mut menu_remove = nwg::MenuItem::default();
    nwg::MenuItem::builder()
        .text("Remove")
        .parent(&list_menu)
        .build(&mut menu_remove)
        .expect("menu remove");

    // ── Notice control + timer ──────────────────────────────────────────
    let mut notice = nwg::Notice::default();
    nwg::Notice::builder()
        .parent(&window)
        .build(&mut notice)
        .expect("notice");

    let mut timer = nwg::AnimationTimer::default();
    nwg::AnimationTimer::builder()
        .parent(&window)
        .interval(std::time::Duration::from_millis(2000))
        .build(&mut timer)
        .expect("timer");
    timer.start();

    // ── Tray icon ───────────────────────────────────────────────────────
    let mut tray_icon_res = nwg::Icon::default();
    let app_icon_loaded = load_embedded_icon(&mut tray_icon_res);
    if !app_icon_loaded {
        let _ = nwg::Icon::builder()
            .source_system(Some(nwg::OemIcon::Information))
            .build(&mut tray_icon_res);
    }
    window.set_icon(Some(&tray_icon_res));

    let mut tray = nwg::TrayNotification::default();
    nwg::TrayNotification::builder()
        .parent(&window)
        .icon(Some(&tray_icon_res))
        .tip(Some("RAM Limiter"))
        .build(&mut tray)
        .expect("tray");

    let mut tray_menu = nwg::Menu::default();
    nwg::Menu::builder()
        .popup(true)
        .parent(&window)
        .build(&mut tray_menu)
        .expect("tray menu");

    let mut tray_open = nwg::MenuItem::default();
    nwg::MenuItem::builder()
        .text("Open")
        .parent(&tray_menu)
        .build(&mut tray_open)
        .expect("tray open");

    let mut tray_exit = nwg::MenuItem::default();
    nwg::MenuItem::builder()
        .text("Exit")
        .parent(&tray_menu)
        .build(&mut tray_exit)
        .expect("tray exit");

    // ── Shared state ────────────────────────────────────────────────────
    let limiter_state = Arc::new(Mutex::new(LimiterState::default()));

    // Build initial rows from config, extracting icons.
    let mut rows: Vec<AppRow> = Vec::new();
    let mut icon_map: HashMap<String, i32> = HashMap::new();
    for entry in config.apps.values() {
        let icon_path = icon_extract::extract_exe_icon(&entry.exe_path);
        if let Some(ref ip) = icon_path {
            if let Ok(idx) = image_list.add_icon_from_filename(&ip.to_string_lossy()) {
                icon_map.insert(entry.exe_path.clone(), idx);
            }
        }
        rows.push(AppRow {
            exe_path: entry.exe_path.clone(),
            display_name: entry.display_name.clone(),
            enabled: entry.enabled,
            soft_limit_mb: entry.soft_limit_mb,
            icon_path,
        });
    }
    list_view.set_image_list(Some(&image_list), nwg::ListViewImageListType::Small);

    // Push initial settings into the limiter state.
    {
        let mut st = limiter_state.lock().unwrap();
        st.global_enabled = config.global_enabled;
        st.skip_focused = config.skip_focused;
        st.trim_interval_ms = config.trim_interval_ms;
        st.triggers = config
            .triggers
            .iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect::<HashSet<_>>();
        for row in &rows {
            let proc = icon_extract::process_name_from_path(&row.exe_path).to_lowercase();
            if proc.is_empty() {
                continue;
            }
            st.apps.insert(
                proc,
                AppRuntime {
                    display_name: row.display_name.clone(),
                    enabled: row.enabled,
                    soft_limit_mb: row.soft_limit_mb,
                },
            );
        }
    }

    let state = Rc::new(RefCell::new(AppState {
        config,
        rows,
        limiter: Arc::clone(&limiter_state),
        editing_idx: None,
    }));
    let icon_map = Rc::new(RefCell::new(icon_map));
    let picker_data: Rc<RefCell<Vec<ProcessInfo>>> = Rc::new(RefCell::new(Vec::new()));
    let current_view = Rc::new(RefCell::new(View::Main));

    // Initial UI population.
    rebuild_app_list(&state.borrow(), &list_view, &icon_map.borrow());
    refresh_status_and_stats(&state.borrow(), &status_label, &stats_label);

    // ── Background worker → Notice ──────────────────────────────────────
    let notice_sender = notice.sender();
    limiter::start_worker(
        Arc::clone(&limiter_state),
        Some(Box::new(move || notice_sender.notice())),
    );

    let limiter_for_exit = Arc::clone(&limiter_state);
    let state_for_cleanup = Rc::clone(&state);

    let handler = nwg::full_bind_event_handler(&window.handle, {
        let window = window.handle;
        let state = Rc::clone(&state);
        let icon_map = Rc::clone(&icon_map);
        let picker_data = Rc::clone(&picker_data);
        let current_view = Rc::clone(&current_view);
        move |evt, _evt_data, handle| {
            use nwg::Event as E;
            match evt {
                // ── Browse for .exe ──────────────────────────────────
                E::OnButtonClick if handle == browse_btn => {
                    let mut file_dialog = nwg::FileDialog::default();
                    let _ = nwg::FileDialog::builder()
                        .title("Select an application")
                        .action(nwg::FileDialogAction::Open)
                        .filters("Executables(*.exe)|All(*.*)")
                        .build(&mut file_dialog);

                    let picked = file_dialog
                        .run(Some(&window))
                        .then(|| file_dialog.get_selected_item().ok())
                        .flatten();
                    if let Some(path_os) = picked {
                        let path = path_os.to_string_lossy().to_string();
                        let name = icon_extract::process_name_from_path(&path);
                        if !name.is_empty() {
                            add_app_to_state(
                                &state,
                                &icon_map,
                                &image_list,
                                &list_view,
                                path,
                                name,
                            );
                            refresh_status_and_stats(&state.borrow(), &status_label, &stats_label);
                        }
                    }
                }

                // ── Open the picker view ─────────────────────────────
                E::OnButtonClick if handle == pick_btn => {
                    let snapshot = limiter::enumerate_processes();
                    *picker_data.borrow_mut() = snapshot;
                    populate_picker(&picker_list, &picker_data.borrow(), "");
                    *current_view.borrow_mut() = View::Picker;
                    apply_view_visibility(
                        View::Picker,
                        &visibility_handles(
                            &browse_btn,
                            &pick_btn,
                            &profiles_btn,
                            &master_cb,
                            &skip_focus_cb,
                            &interval_label,
                            &interval_input,
                            &triggers_label,
                            &triggers_input,
                            &list_view,
                            &status_label,
                            &stats_label,
                            &autostart_cb,
                            &hint_label,
                            &picker_back,
                            &picker_refresh,
                            &picker_add,
                            &picker_filter,
                            &picker_list,
                            &profile_back,
                            &profile_header,
                            &profile_list,
                            &profile_activate,
                            &profile_delete,
                            &profile_active_label,
                            &profile_save_label,
                            &profile_name_input,
                            &profile_save_btn,
                            &profile_hint,
                            &soft_back,
                            &soft_title,
                            &soft_caption,
                            &soft_input,
                            &soft_save,
                            &soft_clear,
                            &soft_hint,
                        ),
                    );
                    picker_filter.set_text("");
                }

                // ── Picker: Back ─────────────────────────────────────
                E::OnButtonClick if handle == picker_back => {
                    *current_view.borrow_mut() = View::Main;
                    apply_view_visibility(
                        View::Main,
                        &visibility_handles(
                            &browse_btn,
                            &pick_btn,
                            &profiles_btn,
                            &master_cb,
                            &skip_focus_cb,
                            &interval_label,
                            &interval_input,
                            &triggers_label,
                            &triggers_input,
                            &list_view,
                            &status_label,
                            &stats_label,
                            &autostart_cb,
                            &hint_label,
                            &picker_back,
                            &picker_refresh,
                            &picker_add,
                            &picker_filter,
                            &picker_list,
                            &profile_back,
                            &profile_header,
                            &profile_list,
                            &profile_activate,
                            &profile_delete,
                            &profile_active_label,
                            &profile_save_label,
                            &profile_name_input,
                            &profile_save_btn,
                            &profile_hint,
                            &soft_back,
                            &soft_title,
                            &soft_caption,
                            &soft_input,
                            &soft_save,
                            &soft_clear,
                            &soft_hint,
                        ),
                    );
                }

                // ── Picker: Refresh ──────────────────────────────────
                E::OnButtonClick if handle == picker_refresh => {
                    let snapshot = limiter::enumerate_processes();
                    *picker_data.borrow_mut() = snapshot;
                    populate_picker(&picker_list, &picker_data.borrow(), &picker_filter.text());
                }

                // ── Picker: live filter ──────────────────────────────
                E::OnTextInput if handle == picker_filter => {
                    populate_picker(&picker_list, &picker_data.borrow(), &picker_filter.text());
                }

                // ── Picker: Add (button or double-click) ─────────────
                E::OnButtonClick if handle == picker_add => {
                    if let Some(idx) = picker_list.selected_item() {
                        let data = picker_data.borrow();
                        let q = picker_filter.text();
                        let filtered = filter_processes(&data, &q);
                        if let Some(p) = filtered.get(idx) {
                            commit_picker_choice(p, &state, &icon_map, &image_list, &list_view);
                            refresh_status_and_stats(&state.borrow(), &status_label, &stats_label);
                            *current_view.borrow_mut() = View::Main;
                            apply_view_visibility(
                                View::Main,
                                &visibility_handles(
                                    &browse_btn,
                                    &pick_btn,
                                    &profiles_btn,
                                    &master_cb,
                                    &skip_focus_cb,
                                    &interval_label,
                                    &interval_input,
                                    &triggers_label,
                                    &triggers_input,
                                    &list_view,
                                    &status_label,
                                    &stats_label,
                                    &autostart_cb,
                                    &hint_label,
                                    &picker_back,
                                    &picker_refresh,
                                    &picker_add,
                                    &picker_filter,
                                    &picker_list,
                                    &profile_back,
                                    &profile_header,
                                    &profile_list,
                                    &profile_activate,
                                    &profile_delete,
                                    &profile_active_label,
                                    &profile_save_label,
                                    &profile_name_input,
                                    &profile_save_btn,
                                    &profile_hint,
                                    &soft_back,
                                    &soft_title,
                                    &soft_caption,
                                    &soft_input,
                                    &soft_save,
                                    &soft_clear,
                                    &soft_hint,
                                ),
                            );
                        }
                    }
                }
                E::OnListViewDoubleClick if handle == picker_list => {
                    if let Some(idx) = picker_list.selected_item() {
                        let data = picker_data.borrow();
                        let q = picker_filter.text();
                        let filtered = filter_processes(&data, &q);
                        if let Some(p) = filtered.get(idx) {
                            commit_picker_choice(p, &state, &icon_map, &image_list, &list_view);
                            refresh_status_and_stats(&state.borrow(), &status_label, &stats_label);
                            *current_view.borrow_mut() = View::Main;
                            apply_view_visibility(
                                View::Main,
                                &visibility_handles(
                                    &browse_btn,
                                    &pick_btn,
                                    &profiles_btn,
                                    &master_cb,
                                    &skip_focus_cb,
                                    &interval_label,
                                    &interval_input,
                                    &triggers_label,
                                    &triggers_input,
                                    &list_view,
                                    &status_label,
                                    &stats_label,
                                    &autostart_cb,
                                    &hint_label,
                                    &picker_back,
                                    &picker_refresh,
                                    &picker_add,
                                    &picker_filter,
                                    &picker_list,
                                    &profile_back,
                                    &profile_header,
                                    &profile_list,
                                    &profile_activate,
                                    &profile_delete,
                                    &profile_active_label,
                                    &profile_save_label,
                                    &profile_name_input,
                                    &profile_save_btn,
                                    &profile_hint,
                                    &soft_back,
                                    &soft_title,
                                    &soft_caption,
                                    &soft_input,
                                    &soft_save,
                                    &soft_clear,
                                    &soft_hint,
                                ),
                            );
                        }
                    }
                }

                // ── Open profiles view ───────────────────────────────
                E::OnButtonClick if handle == profiles_btn => {
                    populate_profile_list(&profile_list, &state.borrow().config);
                    update_active_profile_label(
                        &profile_active_label,
                        state.borrow().config.active_profile.as_deref(),
                    );
                    *current_view.borrow_mut() = View::Profiles;
                    apply_view_visibility(
                        View::Profiles,
                        &visibility_handles(
                            &browse_btn,
                            &pick_btn,
                            &profiles_btn,
                            &master_cb,
                            &skip_focus_cb,
                            &interval_label,
                            &interval_input,
                            &triggers_label,
                            &triggers_input,
                            &list_view,
                            &status_label,
                            &stats_label,
                            &autostart_cb,
                            &hint_label,
                            &picker_back,
                            &picker_refresh,
                            &picker_add,
                            &picker_filter,
                            &picker_list,
                            &profile_back,
                            &profile_header,
                            &profile_list,
                            &profile_activate,
                            &profile_delete,
                            &profile_active_label,
                            &profile_save_label,
                            &profile_name_input,
                            &profile_save_btn,
                            &profile_hint,
                            &soft_back,
                            &soft_title,
                            &soft_caption,
                            &soft_input,
                            &soft_save,
                            &soft_clear,
                            &soft_hint,
                        ),
                    );
                }
                E::OnButtonClick if handle == profile_back => {
                    *current_view.borrow_mut() = View::Main;
                    apply_view_visibility(
                        View::Main,
                        &visibility_handles(
                            &browse_btn,
                            &pick_btn,
                            &profiles_btn,
                            &master_cb,
                            &skip_focus_cb,
                            &interval_label,
                            &interval_input,
                            &triggers_label,
                            &triggers_input,
                            &list_view,
                            &status_label,
                            &stats_label,
                            &autostart_cb,
                            &hint_label,
                            &picker_back,
                            &picker_refresh,
                            &picker_add,
                            &picker_filter,
                            &picker_list,
                            &profile_back,
                            &profile_header,
                            &profile_list,
                            &profile_activate,
                            &profile_delete,
                            &profile_active_label,
                            &profile_save_label,
                            &profile_name_input,
                            &profile_save_btn,
                            &profile_hint,
                            &soft_back,
                            &soft_title,
                            &soft_caption,
                            &soft_input,
                            &soft_save,
                            &soft_clear,
                            &soft_hint,
                        ),
                    );
                }
                E::OnButtonClick if handle == profile_save_btn => {
                    let name = profile_name_input.text().trim().to_string();
                    if !name.is_empty() {
                        {
                            let mut st = state.borrow_mut();
                            let snap = st.config.snapshot_profile();
                            st.config.profiles.insert(name.clone(), snap);
                            st.config.active_profile = Some(name.clone());
                            st.config.save();
                        }
                        profile_name_input.set_text("");
                        populate_profile_list(&profile_list, &state.borrow().config);
                        update_active_profile_label(&profile_active_label, Some(&name));
                    }
                }
                E::OnButtonClick if handle == profile_activate => {
                    let selected_name =
                        selected_profile_name(&profile_list, &state.borrow().config);
                    if let Some(name) = selected_name {
                        {
                            let mut st = state.borrow_mut();
                            if let Some(profile) = st.config.profiles.get(&name).cloned() {
                                st.config.apply_profile(&profile);
                                st.config.active_profile = Some(name.clone());
                                st.config.save();
                            }
                        }
                        // Rebuild rows + icon map from the new config.
                        rebuild_rows_from_config(&state, &icon_map, &image_list);
                        rebuild_app_list(&state.borrow(), &list_view, &icon_map.borrow());

                        // Sync UI controls + limiter state.
                        let st = state.borrow();
                        master_cb.set_check_state(if st.config.global_enabled {
                            nwg::CheckBoxState::Checked
                        } else {
                            nwg::CheckBoxState::Unchecked
                        });
                        skip_focus_cb.set_check_state(if st.config.skip_focused {
                            nwg::CheckBoxState::Checked
                        } else {
                            nwg::CheckBoxState::Unchecked
                        });
                        interval_input.set_text(&st.config.trim_interval_ms.to_string());
                        triggers_input.set_text(&st.config.triggers.join(", "));
                        push_settings_to_limiter(&st);
                        push_apps_to_limiter(&st);
                        refresh_status_and_stats(&st, &status_label, &stats_label);
                        update_active_profile_label(
                            &profile_active_label,
                            st.config.active_profile.as_deref(),
                        );
                    }
                }
                E::OnButtonClick if handle == profile_delete => {
                    let selected_name =
                        selected_profile_name(&profile_list, &state.borrow().config);
                    if let Some(name) = selected_name {
                        {
                            let mut st = state.borrow_mut();
                            st.config.profiles.remove(&name);
                            if st.config.active_profile.as_deref() == Some(name.as_str()) {
                                st.config.active_profile = None;
                            }
                            st.config.save();
                        }
                        populate_profile_list(&profile_list, &state.borrow().config);
                        update_active_profile_label(
                            &profile_active_label,
                            state.borrow().config.active_profile.as_deref(),
                        );
                    }
                }

                // ── Soft-limit edit view buttons ─────────────────────
                E::OnButtonClick if handle == soft_back => {
                    state.borrow_mut().editing_idx = None;
                    *current_view.borrow_mut() = View::Main;
                    apply_view_visibility(
                        View::Main,
                        &visibility_handles(
                            &browse_btn,
                            &pick_btn,
                            &profiles_btn,
                            &master_cb,
                            &skip_focus_cb,
                            &interval_label,
                            &interval_input,
                            &triggers_label,
                            &triggers_input,
                            &list_view,
                            &status_label,
                            &stats_label,
                            &autostart_cb,
                            &hint_label,
                            &picker_back,
                            &picker_refresh,
                            &picker_add,
                            &picker_filter,
                            &picker_list,
                            &profile_back,
                            &profile_header,
                            &profile_list,
                            &profile_activate,
                            &profile_delete,
                            &profile_active_label,
                            &profile_save_label,
                            &profile_name_input,
                            &profile_save_btn,
                            &profile_hint,
                            &soft_back,
                            &soft_title,
                            &soft_caption,
                            &soft_input,
                            &soft_save,
                            &soft_clear,
                            &soft_hint,
                        ),
                    );
                }
                E::OnButtonClick if handle == soft_save => {
                    if let Some(idx) = state.borrow().editing_idx {
                        let parsed: Option<u64> = soft_input.text().trim().parse().ok();
                        set_app_soft_limit(&state, idx, parsed);
                        rebuild_app_list(&state.borrow(), &list_view, &icon_map.borrow());
                        refresh_status_and_stats(&state.borrow(), &status_label, &stats_label);
                    }
                    state.borrow_mut().editing_idx = None;
                    *current_view.borrow_mut() = View::Main;
                    apply_view_visibility(
                        View::Main,
                        &visibility_handles(
                            &browse_btn,
                            &pick_btn,
                            &profiles_btn,
                            &master_cb,
                            &skip_focus_cb,
                            &interval_label,
                            &interval_input,
                            &triggers_label,
                            &triggers_input,
                            &list_view,
                            &status_label,
                            &stats_label,
                            &autostart_cb,
                            &hint_label,
                            &picker_back,
                            &picker_refresh,
                            &picker_add,
                            &picker_filter,
                            &picker_list,
                            &profile_back,
                            &profile_header,
                            &profile_list,
                            &profile_activate,
                            &profile_delete,
                            &profile_active_label,
                            &profile_save_label,
                            &profile_name_input,
                            &profile_save_btn,
                            &profile_hint,
                            &soft_back,
                            &soft_title,
                            &soft_caption,
                            &soft_input,
                            &soft_save,
                            &soft_clear,
                            &soft_hint,
                        ),
                    );
                }
                E::OnButtonClick if handle == soft_clear => {
                    if let Some(idx) = state.borrow().editing_idx {
                        set_app_soft_limit(&state, idx, None);
                        rebuild_app_list(&state.borrow(), &list_view, &icon_map.borrow());
                        refresh_status_and_stats(&state.borrow(), &status_label, &stats_label);
                    }
                    soft_input.set_text("");
                }

                // ── Master enabled toggle ────────────────────────────
                E::OnButtonClick if handle == master_cb => {
                    let checked = master_cb.check_state() == nwg::CheckBoxState::Checked;
                    let mut st = state.borrow_mut();
                    st.config.global_enabled = checked;
                    st.config.save();
                    push_settings_to_limiter(&st);
                }

                // ── Skip focused toggle ──────────────────────────────
                E::OnButtonClick if handle == skip_focus_cb => {
                    let checked = skip_focus_cb.check_state() == nwg::CheckBoxState::Checked;
                    let mut st = state.borrow_mut();
                    st.config.skip_focused = checked;
                    st.config.save();
                    push_settings_to_limiter(&st);
                }

                // ── Interval text input ──────────────────────────────
                E::OnTextInput if handle == interval_input => {
                    let raw = interval_input.text();
                    if let Ok(parsed) = raw.trim().parse::<u64>() {
                        let v = parsed.clamp(100, 600_000);
                        let mut st = state.borrow_mut();
                        if st.config.trim_interval_ms != v {
                            st.config.trim_interval_ms = v;
                            st.config.save();
                            push_settings_to_limiter(&st);
                        }
                    }
                }

                // ── Triggers text input ──────────────────────────────
                E::OnTextInput if handle == triggers_input => {
                    let raw = triggers_input.text();
                    let mut st = state.borrow_mut();
                    st.config.triggers = parse_triggers(&raw);
                    st.config.save();
                    push_settings_to_limiter(&st);
                }

                // ── Main list: double-click toggles ON/OFF ───────────
                E::OnListViewDoubleClick if handle == list_view => {
                    if let Some(idx) = list_view.selected_item() {
                        toggle_app_enabled(&state, idx);
                        rebuild_app_list(&state.borrow(), &list_view, &icon_map.borrow());
                        refresh_status_and_stats(&state.borrow(), &status_label, &stats_label);
                    }
                }

                // ── Main list: right-click → context menu ────────────
                E::OnListViewRightClick if handle == list_view => {
                    if list_view.selected_item().is_some() {
                        let (x, y) = nwg::GlobalCursor::position();
                        list_menu.popup(x, y);
                    }
                }

                E::OnMenuItemSelected if handle == menu_toggle => {
                    if let Some(idx) = list_view.selected_item() {
                        toggle_app_enabled(&state, idx);
                        rebuild_app_list(&state.borrow(), &list_view, &icon_map.borrow());
                        refresh_status_and_stats(&state.borrow(), &status_label, &stats_label);
                    }
                }
                E::OnMenuItemSelected if handle == menu_set_soft => {
                    if let Some(idx) = list_view.selected_item() {
                        let st = state.borrow();
                        if idx < st.rows.len() {
                            let row = &st.rows[idx];
                            soft_title.set_text(&format!(
                                "Soft working-set cap for: {}",
                                row.display_name
                            ));
                            soft_input.set_text(
                                &row.soft_limit_mb.map(|v| v.to_string()).unwrap_or_default(),
                            );
                            drop(st);
                            state.borrow_mut().editing_idx = Some(idx);
                            *current_view.borrow_mut() = View::SoftLimit;
                            apply_view_visibility(
                                View::SoftLimit,
                                &visibility_handles(
                                    &browse_btn,
                                    &pick_btn,
                                    &profiles_btn,
                                    &master_cb,
                                    &skip_focus_cb,
                                    &interval_label,
                                    &interval_input,
                                    &triggers_label,
                                    &triggers_input,
                                    &list_view,
                                    &status_label,
                                    &stats_label,
                                    &autostart_cb,
                                    &hint_label,
                                    &picker_back,
                                    &picker_refresh,
                                    &picker_add,
                                    &picker_filter,
                                    &picker_list,
                                    &profile_back,
                                    &profile_header,
                                    &profile_list,
                                    &profile_activate,
                                    &profile_delete,
                                    &profile_active_label,
                                    &profile_save_label,
                                    &profile_name_input,
                                    &profile_save_btn,
                                    &profile_hint,
                                    &soft_back,
                                    &soft_title,
                                    &soft_caption,
                                    &soft_input,
                                    &soft_save,
                                    &soft_clear,
                                    &soft_hint,
                                ),
                            );
                        }
                    }
                }
                E::OnMenuItemSelected if handle == menu_clear_soft => {
                    if let Some(idx) = list_view.selected_item() {
                        set_app_soft_limit(&state, idx, None);
                        rebuild_app_list(&state.borrow(), &list_view, &icon_map.borrow());
                        refresh_status_and_stats(&state.borrow(), &status_label, &stats_label);
                    }
                }
                E::OnMenuItemSelected if handle == menu_remove => {
                    if let Some(idx) = list_view.selected_item() {
                        remove_app(&state, idx);
                        rebuild_app_list(&state.borrow(), &list_view, &icon_map.borrow());
                        refresh_status_and_stats(&state.borrow(), &status_label, &stats_label);
                    }
                }

                // ── Autostart ────────────────────────────────────────
                E::OnButtonClick if handle == autostart_cb => {
                    let checked = autostart_cb.check_state() == nwg::CheckBoxState::Checked;
                    autostart::set_enabled(checked);
                    let mut st = state.borrow_mut();
                    st.config.autostart = checked;
                    st.config.save();
                }

                // ── Timer / Notice → refresh status + stats ─────────
                E::OnTimerTick if handle == timer => {
                    refresh_status_and_stats(&state.borrow(), &status_label, &stats_label);
                }
                E::OnNotice if handle == notice => {
                    refresh_status_and_stats(&state.borrow(), &status_label, &stats_label);
                }

                // ── Tray ─────────────────────────────────────────────
                E::OnContextMenu if handle == tray => {
                    let (x, y) = nwg::GlobalCursor::position();
                    tray_menu.popup(x, y);
                }
                E::OnMenuItemSelected if handle == tray_open => {
                    show_hwnd(&window, true);
                    focus_hwnd(&window);
                }
                E::OnMenuItemSelected if handle == tray_exit => {
                    {
                        let mut ls = limiter_for_exit.lock().unwrap();
                        ls.running = false;
                    }
                    nwg::stop_thread_dispatch();
                }

                // ── Window minimize / close → hide to tray ───────────
                E::OnWindowMinimize if handle == window => {
                    show_hwnd(&window, false);
                }
                E::OnWindowClose if handle == window => {
                    show_hwnd(&window, false);
                }

                _ => {}
            }
        }
    });

    if start_minimized {
        window.set_visible(false);
    }

    nwg::dispatch_thread_events();
    nwg::unbind_event_handler(&handler);
    {
        let st = state_for_cleanup.borrow();
        let mut ls = st.limiter.lock().unwrap();
        ls.running = false;
    }
}

// ── helpers ────────────────────────────────────────────────────────────

fn fmt_mb(bytes: u64) -> String {
    let mb = bytes as f64 / 1024.0 / 1024.0;
    if mb >= 1024.0 {
        format!("{:.2} GB", mb / 1024.0)
    } else if mb >= 10.0 {
        format!("{mb:.0} MB")
    } else {
        format!("{mb:.1} MB")
    }
}

fn fmt_int(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i != 0 && i % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

fn parse_triggers(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || c == ';' || c == '\n')
        .map(|s| s.trim().trim_end_matches(".exe").trim_end_matches(".EXE"))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn refresh_status_and_stats(st: &AppState, status_label: &nwg::Label, stats_label: &nwg::Label) {
    let total = st.rows.len();
    let active = st.rows.iter().filter(|r| r.enabled).count();

    let (working_set_total, processes, page_faults, shed) = {
        let ls = st.limiter.lock().unwrap();
        (
            ls.stats.last_working_set_total,
            ls.stats.last_process_count,
            ls.stats.last_page_faults_total,
            ls.stats.estimated_pagefile_bytes,
        )
    };

    if total == 0 {
        status_label.set_text("No applications added");
    } else if active == 0 {
        status_label.set_text(&format!("{total} app(s) listed • all paused"));
    } else if !st.config.global_enabled {
        status_label.set_text(&format!(
            "{} of {} active • master switch OFF",
            active, total
        ));
    } else {
        status_label.set_text(&format!(
            "RAM: {} across {} process(es) • {} of {} active",
            fmt_mb(working_set_total),
            processes,
            active,
            total
        ));
    }

    stats_label.set_text(&format!(
        "Trimmed: {} (estimated disk writes) · Page faults: {}",
        fmt_mb(shed),
        fmt_int(page_faults)
    ));
}

fn filter_processes<'a>(all: &'a [ProcessInfo], query: &str) -> Vec<&'a ProcessInfo> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        all.iter().collect()
    } else {
        all.iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&q)
                    || p.exe_path.to_lowercase().contains(&q)
                    || p.pid.to_string().contains(&q)
            })
            .collect()
    }
}

fn populate_picker(picker_list: &nwg::ListView, all: &[ProcessInfo], query: &str) {
    picker_list.clear();
    let filtered = filter_processes(all, query);
    for (i, p) in filtered.iter().enumerate() {
        picker_list.insert_item(nwg::InsertListViewItem {
            index: Some(i as i32),
            column_index: 0,
            text: Some(p.name.clone()),
            image: None,
        });
        picker_list.insert_item(nwg::InsertListViewItem {
            index: Some(i as i32),
            column_index: 1,
            text: Some(p.pid.to_string()),
            image: None,
        });
        picker_list.insert_item(nwg::InsertListViewItem {
            index: Some(i as i32),
            column_index: 2,
            text: Some(if p.working_set == 0 {
                "—".into()
            } else {
                fmt_mb(p.working_set)
            }),
            image: None,
        });
        picker_list.insert_item(nwg::InsertListViewItem {
            index: Some(i as i32),
            column_index: 3,
            text: Some(p.exe_path.clone()),
            image: None,
        });
    }
}

fn populate_profile_list(profile_list: &nwg::ListView, config: &Config) {
    profile_list.clear();
    let mut names: Vec<&String> = config.profiles.keys().collect();
    names.sort();
    for (i, name) in names.iter().enumerate() {
        let profile = &config.profiles[*name];
        profile_list.insert_item(nwg::InsertListViewItem {
            index: Some(i as i32),
            column_index: 0,
            text: Some((*name).clone()),
            image: None,
        });
        profile_list.insert_item(nwg::InsertListViewItem {
            index: Some(i as i32),
            column_index: 1,
            text: Some(profile.apps.len().to_string()),
            image: None,
        });
        profile_list.insert_item(nwg::InsertListViewItem {
            index: Some(i as i32),
            column_index: 2,
            text: Some(profile.trim_interval_ms.to_string()),
            image: None,
        });
    }
}

fn selected_profile_name(profile_list: &nwg::ListView, config: &Config) -> Option<String> {
    let idx = profile_list.selected_item()?;
    let mut names: Vec<&String> = config.profiles.keys().collect();
    names.sort();
    names.get(idx).map(|s| (*s).clone())
}

fn update_active_profile_label(label: &nwg::Label, name: Option<&str>) {
    label.set_text(&format!("Active: {}", name.unwrap_or("(none)")));
}

fn add_app_to_state(
    state: &Rc<RefCell<AppState>>,
    icon_map: &Rc<RefCell<HashMap<String, i32>>>,
    image_list: &nwg::ImageList,
    list_view: &nwg::ListView,
    path: String,
    display_name: String,
) {
    {
        let mut st = state.borrow_mut();
        if st.config.apps.contains_key(&path) {
            return;
        }
        let icon_path = icon_extract::extract_exe_icon(&path);
        let mut imap = icon_map.borrow_mut();
        if let Some(ref ip) = icon_path {
            if let Ok(idx) = image_list.add_icon_from_filename(&ip.to_string_lossy()) {
                imap.insert(path.clone(), idx);
            }
        }
        st.rows.push(AppRow {
            exe_path: path.clone(),
            display_name: display_name.clone(),
            enabled: true,
            soft_limit_mb: None,
            icon_path,
        });
        st.config.apps.insert(
            path.clone(),
            AppEntry::new(path.clone(), display_name.clone()),
        );
        push_apps_to_limiter(&st);
        st.config.save();
    }
    rebuild_app_list(&state.borrow(), list_view, &icon_map.borrow());
}

fn commit_picker_choice(
    p: &ProcessInfo,
    state: &Rc<RefCell<AppState>>,
    icon_map: &Rc<RefCell<HashMap<String, i32>>>,
    image_list: &nwg::ImageList,
    list_view: &nwg::ListView,
) {
    let path = if !p.exe_path.is_empty() {
        p.exe_path.clone()
    } else {
        p.name.clone()
    };
    let display_name = p
        .name
        .strip_suffix(".exe")
        .or_else(|| p.name.strip_suffix(".EXE"))
        .map(str::to_owned)
        .unwrap_or_else(|| p.name.clone());
    add_app_to_state(state, icon_map, image_list, list_view, path, display_name);
}

fn toggle_app_enabled(state: &Rc<RefCell<AppState>>, idx: usize) {
    let mut st = state.borrow_mut();
    if idx >= st.rows.len() {
        return;
    }
    st.rows[idx].enabled = !st.rows[idx].enabled;
    let key = st.rows[idx].exe_path.clone();
    let new_enabled = st.rows[idx].enabled;
    if let Some(entry) = st.config.apps.get_mut(&key) {
        entry.enabled = new_enabled;
    }
    st.config.save();
    push_apps_to_limiter(&st);
}

fn set_app_soft_limit(state: &Rc<RefCell<AppState>>, idx: usize, soft: Option<u64>) {
    let mut st = state.borrow_mut();
    if idx >= st.rows.len() {
        return;
    }
    st.rows[idx].soft_limit_mb = soft;
    let key = st.rows[idx].exe_path.clone();
    if let Some(entry) = st.config.apps.get_mut(&key) {
        entry.soft_limit_mb = soft;
    }
    st.config.save();
    push_apps_to_limiter(&st);
}

fn remove_app(state: &Rc<RefCell<AppState>>, idx: usize) {
    let mut st = state.borrow_mut();
    if idx >= st.rows.len() {
        return;
    }
    let key = st.rows[idx].exe_path.clone();
    st.config.apps.remove(&key);
    st.rows.remove(idx);
    st.config.save();
    push_apps_to_limiter(&st);
}

fn push_settings_to_limiter(st: &AppState) {
    let mut ls = st.limiter.lock().unwrap();
    ls.global_enabled = st.config.global_enabled;
    ls.skip_focused = st.config.skip_focused;
    ls.trim_interval_ms = st.config.trim_interval_ms;
    ls.triggers = st
        .config
        .triggers
        .iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect::<HashSet<_>>();
}

fn push_apps_to_limiter(st: &AppState) {
    let mut ls = st.limiter.lock().unwrap();
    ls.apps.clear();
    for row in &st.rows {
        let proc = icon_extract::process_name_from_path(&row.exe_path).to_lowercase();
        if proc.is_empty() {
            continue;
        }
        ls.apps.insert(
            proc,
            AppRuntime {
                display_name: row.display_name.clone(),
                enabled: row.enabled,
                soft_limit_mb: row.soft_limit_mb,
            },
        );
    }
    // Wipe stale per-app stats so the UI doesn't show numbers for removed apps.
    let valid: HashSet<String> = st.rows.iter().map(|r| r.display_name.clone()).collect();
    ls.stats.app_page_faults.retain(|k, _| valid.contains(k));
    ls.stats.app_working_set.retain(|k, _| valid.contains(k));
    ls.stats.app_process_count.retain(|k, _| valid.contains(k));
}

/// Re-extract icons + rows from `state.config.apps`. Used after applying a profile.
fn rebuild_rows_from_config(
    state: &Rc<RefCell<AppState>>,
    icon_map: &Rc<RefCell<HashMap<String, i32>>>,
    image_list: &nwg::ImageList,
) {
    let mut st = state.borrow_mut();
    let mut imap = icon_map.borrow_mut();
    let entries: Vec<AppEntry> = st.config.apps.values().cloned().collect();
    st.rows.clear();
    for entry in entries {
        if !imap.contains_key(&entry.exe_path) {
            if let Some(ip) = icon_extract::extract_exe_icon(&entry.exe_path) {
                if let Ok(idx) = image_list.add_icon_from_filename(&ip.to_string_lossy()) {
                    imap.insert(entry.exe_path.clone(), idx);
                }
            }
        }
        st.rows.push(AppRow {
            exe_path: entry.exe_path,
            display_name: entry.display_name,
            enabled: entry.enabled,
            soft_limit_mb: entry.soft_limit_mb,
            icon_path: None,
        });
    }
}

fn rebuild_app_list(st: &AppState, list_view: &nwg::ListView, icon_map: &HashMap<String, i32>) {
    list_view.clear();
    let stats_snapshot: Option<(
        HashMap<String, u64>,
        HashMap<String, u64>,
        HashMap<String, u32>,
    )> = {
        let ls = st.limiter.lock().ok().map(|g| {
            (
                g.stats.app_working_set.clone(),
                g.stats.app_page_faults.clone(),
                g.stats.app_process_count.clone(),
            )
        });
        ls
    };

    for (i, row) in st.rows.iter().enumerate() {
        let img_idx = icon_map.get(&row.exe_path).copied();
        list_view.insert_item(nwg::InsertListViewItem {
            index: Some(i as i32),
            column_index: 0,
            text: Some(row.display_name.clone()),
            image: img_idx,
        });
        list_view.insert_item(nwg::InsertListViewItem {
            index: Some(i as i32),
            column_index: 1,
            text: Some(if row.enabled {
                "ON".into()
            } else {
                "OFF".into()
            }),
            image: None,
        });
        list_view.insert_item(nwg::InsertListViewItem {
            index: Some(i as i32),
            column_index: 2,
            text: Some(match row.soft_limit_mb {
                Some(v) => format!("{v} MB"),
                None => "trim".into(),
            }),
            image: None,
        });

        // RAM / page faults from the latest stats snapshot.
        let (ws_text, pf_text) = match &stats_snapshot {
            Some((ws, pf, pc)) => {
                let bytes = ws.get(&row.display_name).copied().unwrap_or(0);
                let faults = pf.get(&row.display_name).copied().unwrap_or(0);
                let count = pc.get(&row.display_name).copied().unwrap_or(0);
                let ws_t = if count == 0 {
                    "—".to_string()
                } else {
                    fmt_mb(bytes)
                };
                let pf_t = if count == 0 {
                    "—".to_string()
                } else if count == 1 {
                    fmt_int(faults)
                } else {
                    format!("{} ({} procs)", fmt_int(faults), count)
                };
                (ws_t, pf_t)
            }
            None => ("—".to_string(), "—".to_string()),
        };
        list_view.insert_item(nwg::InsertListViewItem {
            index: Some(i as i32),
            column_index: 3,
            text: Some(ws_text),
            image: None,
        });
        list_view.insert_item(nwg::InsertListViewItem {
            index: Some(i as i32),
            column_index: 4,
            text: Some(pf_text),
            image: None,
        });
    }
}

// ── view-visibility plumbing ───────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
struct ViewHandles<'a> {
    // Main view widgets
    browse_btn: &'a nwg::Button,
    pick_btn: &'a nwg::Button,
    profiles_btn: &'a nwg::Button,
    master_cb: &'a nwg::CheckBox,
    skip_focus_cb: &'a nwg::CheckBox,
    interval_label: &'a nwg::Label,
    interval_input: &'a nwg::TextInput,
    triggers_label: &'a nwg::Label,
    triggers_input: &'a nwg::TextInput,
    list_view: &'a nwg::ListView,
    status_label: &'a nwg::Label,
    stats_label: &'a nwg::Label,
    autostart_cb: &'a nwg::CheckBox,
    hint_label: &'a nwg::Label,

    // Picker view widgets
    picker_back: &'a nwg::Button,
    picker_refresh: &'a nwg::Button,
    picker_add: &'a nwg::Button,
    picker_filter: &'a nwg::TextInput,
    picker_list: &'a nwg::ListView,

    // Profiles view widgets
    profile_back: &'a nwg::Button,
    profile_header: &'a nwg::Label,
    profile_list: &'a nwg::ListView,
    profile_activate: &'a nwg::Button,
    profile_delete: &'a nwg::Button,
    profile_active_label: &'a nwg::Label,
    profile_save_label: &'a nwg::Label,
    profile_name_input: &'a nwg::TextInput,
    profile_save_btn: &'a nwg::Button,
    profile_hint: &'a nwg::Label,

    // Soft-limit view widgets
    soft_back: &'a nwg::Button,
    soft_title: &'a nwg::Label,
    soft_caption: &'a nwg::Label,
    soft_input: &'a nwg::TextInput,
    soft_save: &'a nwg::Button,
    soft_clear: &'a nwg::Button,
    soft_hint: &'a nwg::Label,
}

#[allow(clippy::too_many_arguments)]
fn visibility_handles<'a>(
    browse_btn: &'a nwg::Button,
    pick_btn: &'a nwg::Button,
    profiles_btn: &'a nwg::Button,
    master_cb: &'a nwg::CheckBox,
    skip_focus_cb: &'a nwg::CheckBox,
    interval_label: &'a nwg::Label,
    interval_input: &'a nwg::TextInput,
    triggers_label: &'a nwg::Label,
    triggers_input: &'a nwg::TextInput,
    list_view: &'a nwg::ListView,
    status_label: &'a nwg::Label,
    stats_label: &'a nwg::Label,
    autostart_cb: &'a nwg::CheckBox,
    hint_label: &'a nwg::Label,
    picker_back: &'a nwg::Button,
    picker_refresh: &'a nwg::Button,
    picker_add: &'a nwg::Button,
    picker_filter: &'a nwg::TextInput,
    picker_list: &'a nwg::ListView,
    profile_back: &'a nwg::Button,
    profile_header: &'a nwg::Label,
    profile_list: &'a nwg::ListView,
    profile_activate: &'a nwg::Button,
    profile_delete: &'a nwg::Button,
    profile_active_label: &'a nwg::Label,
    profile_save_label: &'a nwg::Label,
    profile_name_input: &'a nwg::TextInput,
    profile_save_btn: &'a nwg::Button,
    profile_hint: &'a nwg::Label,
    soft_back: &'a nwg::Button,
    soft_title: &'a nwg::Label,
    soft_caption: &'a nwg::Label,
    soft_input: &'a nwg::TextInput,
    soft_save: &'a nwg::Button,
    soft_clear: &'a nwg::Button,
    soft_hint: &'a nwg::Label,
) -> ViewHandles<'a> {
    ViewHandles {
        browse_btn,
        pick_btn,
        profiles_btn,
        master_cb,
        skip_focus_cb,
        interval_label,
        interval_input,
        triggers_label,
        triggers_input,
        list_view,
        status_label,
        stats_label,
        autostart_cb,
        hint_label,
        picker_back,
        picker_refresh,
        picker_add,
        picker_filter,
        picker_list,
        profile_back,
        profile_header,
        profile_list,
        profile_activate,
        profile_delete,
        profile_active_label,
        profile_save_label,
        profile_name_input,
        profile_save_btn,
        profile_hint,
        soft_back,
        soft_title,
        soft_caption,
        soft_input,
        soft_save,
        soft_clear,
        soft_hint,
    }
}

fn apply_view_visibility(view: View, h: &ViewHandles) {
    let main = view == View::Main;
    let picker = view == View::Picker;
    let profiles = view == View::Profiles;
    let soft = view == View::SoftLimit;

    show_hwnd(&h.browse_btn.handle, main);
    show_hwnd(&h.pick_btn.handle, main);
    show_hwnd(&h.profiles_btn.handle, main);
    show_hwnd(&h.master_cb.handle, main);
    show_hwnd(&h.skip_focus_cb.handle, main);
    show_hwnd(&h.interval_label.handle, main);
    show_hwnd(&h.interval_input.handle, main);
    show_hwnd(&h.triggers_label.handle, main);
    show_hwnd(&h.triggers_input.handle, main);
    show_hwnd(&h.list_view.handle, main);
    show_hwnd(&h.status_label.handle, main);
    show_hwnd(&h.stats_label.handle, main);
    show_hwnd(&h.autostart_cb.handle, main);
    show_hwnd(&h.hint_label.handle, main);

    show_hwnd(&h.picker_back.handle, picker);
    show_hwnd(&h.picker_refresh.handle, picker);
    show_hwnd(&h.picker_add.handle, picker);
    show_hwnd(&h.picker_filter.handle, picker);
    show_hwnd(&h.picker_list.handle, picker);

    show_hwnd(&h.profile_back.handle, profiles);
    show_hwnd(&h.profile_header.handle, profiles);
    show_hwnd(&h.profile_list.handle, profiles);
    show_hwnd(&h.profile_activate.handle, profiles);
    show_hwnd(&h.profile_delete.handle, profiles);
    show_hwnd(&h.profile_active_label.handle, profiles);
    show_hwnd(&h.profile_save_label.handle, profiles);
    show_hwnd(&h.profile_name_input.handle, profiles);
    show_hwnd(&h.profile_save_btn.handle, profiles);
    show_hwnd(&h.profile_hint.handle, profiles);

    show_hwnd(&h.soft_back.handle, soft);
    show_hwnd(&h.soft_title.handle, soft);
    show_hwnd(&h.soft_caption.handle, soft);
    show_hwnd(&h.soft_input.handle, soft);
    show_hwnd(&h.soft_save.handle, soft);
    show_hwnd(&h.soft_clear.handle, soft);
    show_hwnd(&h.soft_hint.handle, soft);
}

/// Try to load the icon embedded as resource id 1 (the icon set via
/// `winresource` in build.rs) into the supplied `nwg::Icon`. Returns false on
/// failure so the caller can fall back to a system icon.
#[cfg(target_os = "windows")]
fn load_embedded_icon(out: &mut nwg::Icon) -> bool {
    use winapi::um::libloaderapi::GetModuleHandleW;
    use winapi::um::winuser::{LoadImageW, IMAGE_ICON, LR_DEFAULTSIZE, LR_SHARED};

    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        #[allow(clippy::manual_dangling_ptr)]
        let resource_id = 1usize as *const u16;
        let h = LoadImageW(
            hinst,
            resource_id,
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE | LR_SHARED,
        );
        if h.is_null() {
            return false;
        }
        out.handle = h as winapi::um::winnt::HANDLE;
        true
    }
}

#[cfg(not(target_os = "windows"))]
fn load_embedded_icon(_out: &mut nwg::Icon) -> bool {
    false
}
