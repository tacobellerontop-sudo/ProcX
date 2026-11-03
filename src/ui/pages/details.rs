use crate::ui::components::not_implemented;

pub fn show(ui: &mut egui::Ui) {
    not_implemented(
        ui,
        "Details",
        "Phase 2",
        &[
            "General: path, command line, creation time, user, architecture, session",
            "Performance: working set, private bytes, peak memory, threads, handles",
            "Threads, loaded modules, parent and child processes",
        ],
    );
}
