use crate::ui::components::not_implemented;

pub fn show(ui: &mut egui::Ui) {
    not_implemented(
        ui,
        "Startup",
        "Phase 4",
        &[
            "User and common startup folders",
            "Run registry keys",
            "Startup scheduled tasks where feasible",
            "Enable/disable supported entries, with confirmation",
        ],
    );
}
