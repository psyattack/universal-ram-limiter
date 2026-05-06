#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod icon_extract;
mod limiter;
mod ui;

fn main() {
    let start_minimized = std::env::args().any(|a| a == "--minimized");
    ui::run(start_minimized);
}
