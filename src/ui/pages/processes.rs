use crate::ui::components::not_implemented;

pub fn show(ui: &mut egui::Ui) {
    not_implemented(
        ui,
        "Processes",
        "Phase 2",
        &[
            "Process table with PID, parent, CPU, memory, threads, handles, user, path, architecture",
            "Sorting, filtering, PID search, column visibility and resizing",
            "Process tree with orphan detection",
            "Context menu actions and confirmed, safeguarded termination",
        ],
    );
}
