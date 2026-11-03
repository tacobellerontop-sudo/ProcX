use crate::ui::components::not_implemented;

pub fn show(ui: &mut egui::Ui) {
    not_implemented(
        ui,
        "History",
        "Phase 5",
        &[
            "SQLite-backed recording keyed by process instance (PID + creation time)",
            "Historical CPU and memory charts, process lifetimes",
            "Retention cleanup and CSV export",
        ],
    );
}
