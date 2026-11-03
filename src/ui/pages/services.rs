use crate::ui::components::not_implemented;

pub fn show(ui: &mut egui::Ui) {
    not_implemented(
        ui,
        "Services",
        "Phase 4",
        &[
            "Service list with state, start type and process ID",
            "Search, running/stopped filter, dependencies",
            "Start/stop where authorized, with confirmation",
        ],
    );
}
