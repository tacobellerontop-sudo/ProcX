//! ProcX — advanced Windows process and system monitor.

// Release builds are GUI-only on Windows (no console window).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod core;
mod logging;
mod monitoring;
mod platform;
mod process;
mod ui;
mod utils;

use crate::core::config::Config;

fn main() -> eframe::Result {
    let config_path = utils::paths::config_file();
    let loaded = Config::load_or_default(&config_path);
    logging::init(loaded.config.logging.level, &utils::paths::log_file());
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "ProcX starting");
    for warning in &loaded.warnings {
        tracing::warn!("{warning}");
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ProcX")
            .with_app_id("procx")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 500.0]),
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "ProcX",
        options,
        Box::new(move |cc| Ok(Box::new(app::ProcxApp::new(cc, loaded, config_path)))),
    )
}
