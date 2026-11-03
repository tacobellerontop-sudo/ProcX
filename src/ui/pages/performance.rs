use crate::ui::components::not_implemented;

pub fn show(ui: &mut egui::Ui) {
    not_implemented(
        ui,
        "Performance",
        "Phase 3",
        &[
            "Per-core CPU utilization and frequency",
            "Memory, page file and commit history",
            "Per-disk throughput and active time",
            "Per-interface network throughput",
            "Configurable chart time ranges",
        ],
    );
}
