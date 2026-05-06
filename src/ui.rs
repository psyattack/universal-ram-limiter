//! Native Windows GUI for the universal RAM Limiter.
//!
//! Compact light-themed layout:
//!  ┌──────────────────────────────────────────────┐
//!  │  RAM Limiter                                 │
//!  │ [+ Browse .exe…] [⌕ Pick running process…]   │
//!  │ ┌─────────────────────────────────────────┐  │
//!  │ │ Application       Status     Action     │  │
//!  │ │ chrome.exe        ON         [Remove]   │  │
//!  │ │ discord.exe       OFF        [Remove]   │  │
//!  │ └─────────────────────────────────────────┘  │
//!  │ Status line • Active count                   │
//!  │ ☐ Launch at startup                          │
//!  │ Hint                                         │
//!  └──────────────────────────────────────────────┘
//!
//! The picker view shares the same window: a search box plus a Detailed
//! ListView of every running process (name / PID / RAM / path) with live
//! filtering. It is shown by hiding the main controls and revealing the
//! picker controls in place.

use crate::config::{autostart, AppEntry, Config};
use crate::icon_extract;
use crate::limiter::{self, LimiterState, ProcessInfo};
use native_windows_gui as nwg;
use std::cell::RefCell;
use std::collections::HashMap;
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

/// Runtime state for a single row in the apps list.
struct AppRow {
    exe_path: String,
    display_name: String,
    enabled: bool,
    #[allow(dead_code)]
    icon_path: Option<std::path::PathBuf>,
}

struct AppState {
    config: Config,
    rows: Vec<AppRow>,
    limiter: Arc<Mutex<LimiterState>>,
}

pub fn run(start_minimized: bool) {
    nwg::init().expect("Failed to init NWG");
    nwg::Font::set_global_family("Segoe UI").expect("Failed to set font");

    let config = Config::load();

    // ── Window ──────────────────────────────────────────────────────────
    let mut window = nwg::Window::default();
    nwg::Window::builder()
        .size((520, 460))
        .position((300, 150))
        .title("RAM Limiter")
        .flags(
            nwg::WindowFlags::WINDOW | nwg::WindowFlags::VISIBLE | nwg::WindowFlags::MINIMIZE_BOX,
        )
        .build(&mut window)
        .expect("Window");

    // Toolbar buttons.
    let mut browse_btn = nwg::Button::default();
    nwg::Button::builder()
        .text("Browse .exe…")
        .size((150, 32))
        .position((12, 12))
        .parent(&window)
        .build(&mut browse_btn)
        .expect("browse btn");

    let mut pick_btn = nwg::Button::default();
    nwg::Button::builder()
        .text("Pick running process…")
        .size((180, 32))
        .position((168, 12))
        .parent(&window)
        .build(&mut pick_btn)
        .expect("pick btn");

    let mut file_dialog = nwg::FileDialog::default();
    nwg::FileDialog::builder()
        .title("Select an application")
        .action(nwg::FileDialogAction::Open)
        .filters("Executables(*.exe)|All(*.*)")
        .build(&mut file_dialog)
        .expect("file dialog");

    // Main apps list.
    let mut list_view = nwg::ListView::default();
    nwg::ListView::builder()
        .parent(&window)
        .size((496, 280))
        .position((12, 52))
        .list_style(nwg::ListViewStyle::Detailed)
        .flags(nwg::ListViewFlags::VISIBLE | nwg::ListViewFlags::SINGLE_SELECTION)
        .ex_flags(nwg::ListViewExFlags::FULL_ROW_SELECT | nwg::ListViewExFlags::GRID)
        .build(&mut list_view)
        .expect("list view");
    list_view.set_headers_enabled(true);
    list_view.insert_column(nwg::InsertListViewColumn {
        index: Some(0),
        fmt: None,
        width: Some(280),
        text: Some("Application".to_string()),
    });
    list_view.insert_column(nwg::InsertListViewColumn {
        index: Some(1),
        fmt: None,
        width: Some(90),
        text: Some("Status".to_string()),
    });
    list_view.insert_column(nwg::InsertListViewColumn {
        index: Some(2),
        fmt: None,
        width: Some(110),
        text: Some("Action".to_string()),
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
        .size((80, 32))
        .position((12, 12))
        .parent(&window)
        .build(&mut picker_back)
        .expect("picker back");

    let mut picker_refresh = nwg::Button::default();
    nwg::Button::builder()
        .text("Refresh")
        .size((80, 32))
        .position((96, 12))
        .parent(&window)
        .build(&mut picker_refresh)
        .expect("picker refresh");

    let mut picker_add = nwg::Button::default();
    nwg::Button::builder()
        .text("Add selected")
        .size((110, 32))
        .position((180, 12))
        .parent(&window)
        .build(&mut picker_add)
        .expect("picker add");

    let mut picker_filter = nwg::TextInput::default();
    nwg::TextInput::builder()
        .text("")
        .placeholder_text(Some("Filter by name or PID…"))
        .size((216, 32))
        .position((294, 12))
        .parent(&window)
        .build(&mut picker_filter)
        .expect("picker filter");

    let mut picker_list = nwg::ListView::default();
    nwg::ListView::builder()
        .parent(&window)
        .size((496, 330))
        .position((12, 52))
        .list_style(nwg::ListViewStyle::Detailed)
        .flags(nwg::ListViewFlags::VISIBLE | nwg::ListViewFlags::SINGLE_SELECTION)
        .ex_flags(nwg::ListViewExFlags::FULL_ROW_SELECT | nwg::ListViewExFlags::GRID)
        .build(&mut picker_list)
        .expect("picker list");
    picker_list.set_headers_enabled(true);
    picker_list.insert_column(nwg::InsertListViewColumn {
        index: Some(0),
        fmt: None,
        width: Some(160),
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
        width: Some(180),
        text: Some("Path".to_string()),
    });

    // Hide picker controls initially.
    show_hwnd(&picker_back.handle, false);
    show_hwnd(&picker_refresh.handle, false);
    show_hwnd(&picker_add.handle, false);
    show_hwnd(&picker_filter.handle, false);
    show_hwnd(&picker_list.handle, false);

    // ── Status / footer ─────────────────────────────────────────────────
    let mut status_label = nwg::Label::default();
    nwg::Label::builder()
        .text("No applications added")
        .size((496, 22))
        .position((12, 340))
        .parent(&window)
        .build(&mut status_label)
        .expect("status label");

    let mut autostart_cb = nwg::CheckBox::default();
    nwg::CheckBox::builder()
        .text("Launch at startup")
        .size((180, 26))
        .position((12, 368))
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
        .text("Double-click row = toggle  •  Right-click = remove")
        .size((496, 22))
        .position((12, 400))
        .parent(&window)
        .build(&mut hint_label)
        .expect("hint label");

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

    // ── Tray icon: prefer the embedded app icon, fall back to system. ───
    let mut tray_icon_res = nwg::Icon::default();
    let app_icon_loaded = load_embedded_icon(&mut tray_icon_res);
    if !app_icon_loaded {
        let _ = nwg::Icon::builder()
            .source_system(Some(nwg::OemIcon::Information))
            .build(&mut tray_icon_res);
    }

    // Apply embedded icon to the window itself.
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
            icon_path,
        });
    }
    list_view.set_image_list(Some(&image_list), nwg::ListViewImageListType::Small);

    for (i, row) in rows.iter().enumerate() {
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
            text: Some("[Remove]".into()),
            image: None,
        });
    }

    // Sync limiter targets.
    {
        let mut st = limiter_state.lock().unwrap();
        for row in &rows {
            if row.enabled {
                st.targets
                    .insert(icon_extract::process_name_from_path(&row.exe_path));
            }
        }
    }

    // Background worker → Notice.
    let notice_sender = notice.sender();
    limiter::start_worker(
        Arc::clone(&limiter_state),
        Some(Box::new(move || notice_sender.notice())),
    );

    let state = Rc::new(RefCell::new(AppState {
        config,
        rows,
        limiter: Arc::clone(&limiter_state),
    }));
    let icon_map = Rc::new(RefCell::new(icon_map));

    // Picker state (refreshed each time the user opens the picker).
    let picker_data: Rc<RefCell<Vec<ProcessInfo>>> = Rc::new(RefCell::new(Vec::new()));

    let limiter_for_exit = Arc::clone(&limiter_state);
    let state_for_cleanup = Rc::clone(&state);

    let handler = nwg::full_bind_event_handler(&window.handle, {
        let window = window.handle;
        move |evt, _evt_data, handle| {
            use nwg::Event as E;
            match evt {
                // ── Browse for .exe ──────────────────────────────────
                E::OnButtonClick if handle == browse_btn => {
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
                        }
                    }
                }

                // ── Open the picker view ─────────────────────────────
                E::OnButtonClick if handle == pick_btn => {
                    let snapshot = limiter::enumerate_processes();
                    *picker_data.borrow_mut() = snapshot;
                    populate_picker(&picker_list, &picker_data.borrow(), "");
                    set_picker_visible(
                        true,
                        &browse_btn,
                        &pick_btn,
                        &list_view,
                        &status_label,
                        &autostart_cb,
                        &hint_label,
                        &picker_back,
                        &picker_refresh,
                        &picker_add,
                        &picker_filter,
                        &picker_list,
                    );
                    picker_filter.set_text("");
                }

                // ── Picker: Back ─────────────────────────────────────
                E::OnButtonClick if handle == picker_back => {
                    set_picker_visible(
                        false,
                        &browse_btn,
                        &pick_btn,
                        &list_view,
                        &status_label,
                        &autostart_cb,
                        &hint_label,
                        &picker_back,
                        &picker_refresh,
                        &picker_add,
                        &picker_filter,
                        &picker_list,
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

                // ── Picker: Add (from button or double-click) ────────
                E::OnButtonClick if handle == picker_add => {
                    if let Some(idx) = picker_list.selected_item() {
                        let data = picker_data.borrow();
                        let q = picker_filter.text();
                        let filtered = filter_processes(&data, &q);
                        if let Some(p) = filtered.get(idx) {
                            commit_picker_choice(p, &state, &icon_map, &image_list, &list_view);
                            set_picker_visible(
                                false,
                                &browse_btn,
                                &pick_btn,
                                &list_view,
                                &status_label,
                                &autostart_cb,
                                &hint_label,
                                &picker_back,
                                &picker_refresh,
                                &picker_add,
                                &picker_filter,
                                &picker_list,
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
                            set_picker_visible(
                                false,
                                &browse_btn,
                                &pick_btn,
                                &list_view,
                                &status_label,
                                &autostart_cb,
                                &hint_label,
                                &picker_back,
                                &picker_refresh,
                                &picker_add,
                                &picker_filter,
                                &picker_list,
                            );
                        }
                    }
                }

                // ── Main list: double-click toggles ON/OFF ───────────
                E::OnListViewDoubleClick if handle == list_view => {
                    if let Some(idx) = list_view.selected_item() {
                        let mut st = state.borrow_mut();
                        if idx < st.rows.len() {
                            st.rows[idx].enabled = !st.rows[idx].enabled;
                            let enabled = st.rows[idx].enabled;
                            let proc_name =
                                icon_extract::process_name_from_path(&st.rows[idx].exe_path);

                            list_view.update_item(
                                idx,
                                nwg::InsertListViewItem {
                                    index: Some(idx as i32),
                                    column_index: 1,
                                    text: Some(if enabled { "ON".into() } else { "OFF".into() }),
                                    image: None,
                                },
                            );

                            {
                                let mut ls = st.limiter.lock().unwrap();
                                if enabled {
                                    ls.targets.insert(proc_name.clone());
                                } else {
                                    ls.targets.remove(&proc_name);
                                }
                            }

                            let key = st.rows[idx].exe_path.clone();
                            if let Some(entry) = st.config.apps.get_mut(&key) {
                                entry.enabled = enabled;
                            }
                            st.config.save();
                        }
                    }
                }

                // ── Main list: right-click removes ───────────────────
                E::OnListViewRightClick if handle == list_view => {
                    if let Some(idx) = list_view.selected_item() {
                        let mut st = state.borrow_mut();
                        if idx < st.rows.len() {
                            let proc_name =
                                icon_extract::process_name_from_path(&st.rows[idx].exe_path);
                            {
                                let mut ls = st.limiter.lock().unwrap();
                                ls.targets.remove(&proc_name);
                            }

                            let key = st.rows[idx].exe_path.clone();
                            st.config.apps.remove(&key);
                            st.rows.remove(idx);
                            st.config.save();

                            list_view.clear();
                            let imap = icon_map.borrow();
                            for (i, row) in st.rows.iter().enumerate() {
                                let img_idx = imap.get(&row.exe_path).copied();
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
                                    text: Some("[Remove]".into()),
                                    image: None,
                                });
                            }
                        }
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

                // ── Timer / Notice → refresh status line ─────────────
                E::OnTimerTick if handle == timer => {
                    refresh_status(&state, &status_label);
                }
                E::OnNotice if handle == notice => {
                    refresh_status(&state, &status_label);
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

fn refresh_status(state: &Rc<RefCell<AppState>>, status_label: &nwg::Label) {
    let st = state.borrow();
    let names: Vec<String> = st
        .rows
        .iter()
        .filter(|r| r.enabled)
        .map(|r| icon_extract::process_name_from_path(&r.exe_path))
        .collect();
    let active_count = st.rows.iter().filter(|r| r.enabled).count();
    let total = st.rows.len();
    drop(st);

    if total == 0 {
        status_label.set_text("No applications added");
    } else if names.is_empty() {
        status_label.set_text(&format!("{total} app(s) listed • all paused"));
    } else {
        let (bytes, procs) = limiter::get_working_set_bytes(&names);
        status_label.set_text(&format!(
            "RAM: {} across {} process(es) • {} of {} active",
            fmt_mb(bytes),
            procs,
            active_count,
            total
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn set_picker_visible(
    show_picker: bool,
    browse_btn: &nwg::Button,
    pick_btn: &nwg::Button,
    list_view: &nwg::ListView,
    status_label: &nwg::Label,
    autostart_cb: &nwg::CheckBox,
    hint_label: &nwg::Label,
    picker_back: &nwg::Button,
    picker_refresh: &nwg::Button,
    picker_add: &nwg::Button,
    picker_filter: &nwg::TextInput,
    picker_list: &nwg::ListView,
) {
    let main_visible = !show_picker;
    show_hwnd(&browse_btn.handle, main_visible);
    show_hwnd(&pick_btn.handle, main_visible);
    show_hwnd(&list_view.handle, main_visible);
    show_hwnd(&status_label.handle, main_visible);
    show_hwnd(&autostart_cb.handle, main_visible);
    show_hwnd(&hint_label.handle, main_visible);

    show_hwnd(&picker_back.handle, show_picker);
    show_hwnd(&picker_refresh.handle, show_picker);
    show_hwnd(&picker_add.handle, show_picker);
    show_hwnd(&picker_filter.handle, show_picker);
    show_hwnd(&picker_list.handle, show_picker);
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

fn add_app_to_state(
    state: &Rc<RefCell<AppState>>,
    icon_map: &Rc<RefCell<HashMap<String, i32>>>,
    image_list: &nwg::ImageList,
    list_view: &nwg::ListView,
    path: String,
    display_name: String,
) {
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

    let row_idx = st.rows.len();
    st.rows.push(AppRow {
        exe_path: path.clone(),
        display_name: display_name.clone(),
        enabled: true,
        icon_path,
    });

    let img_idx = imap.get(&path).copied();
    list_view.insert_item(nwg::InsertListViewItem {
        index: Some(row_idx as i32),
        column_index: 0,
        text: Some(display_name.clone()),
        image: img_idx,
    });
    list_view.insert_item(nwg::InsertListViewItem {
        index: Some(row_idx as i32),
        column_index: 1,
        text: Some("ON".into()),
        image: None,
    });
    list_view.insert_item(nwg::InsertListViewItem {
        index: Some(row_idx as i32),
        column_index: 2,
        text: Some("[Remove]".into()),
        image: None,
    });

    {
        let proc_name = icon_extract::process_name_from_path(&path);
        let mut ls = st.limiter.lock().unwrap();
        ls.targets.insert(proc_name);
    }

    st.config.apps.insert(
        path.clone(),
        AppEntry {
            exe_path: path,
            display_name,
            enabled: true,
        },
    );
    st.config.save();
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

/// Try to load the icon embedded as resource id 1 (the icon set via
/// `winresource` in build.rs) into the supplied `nwg::Icon`. Returns false on
/// failure so the caller can fall back to a system icon.
///
/// `nwg::Icon::handle` is a public field, so we can assign a raw HICON
/// obtained from `LoadImageW` directly. We pass `LR_SHARED`, which means the
/// OS owns the handle and we leave `owned: false` (the Default).
#[cfg(target_os = "windows")]
fn load_embedded_icon(out: &mut nwg::Icon) -> bool {
    use winapi::um::libloaderapi::GetModuleHandleW;
    use winapi::um::winuser::{LoadImageW, IMAGE_ICON, LR_DEFAULTSIZE, LR_SHARED};

    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        // MAKEINTRESOURCEW(1) — Win32 encodes integer resource IDs as a
        // pointer whose high word is zero; this is *not* a real pointer and
        // is the documented way to call LoadImageW with an integer ID.
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
