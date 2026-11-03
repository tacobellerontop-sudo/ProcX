//! Reusable UI building blocks.

pub mod charts;
pub mod dialogs;
pub mod metrics;
pub mod process_table;
pub mod sidebar;
pub mod tables;

use egui::{Frame, Margin, RichText, Stroke};

use super::theme::Palette;

/// A bordered content section with a compact title row.
pub fn section<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let palette = Palette::of(ui);
    Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(3)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().color(palette.muted).small());
            ui.add_space(2.0);
            add(ui)
        })
        .inner
}

/// Page title with an optional right-aligned subtitle.
pub fn page_header(ui: &mut egui::Ui, title: &str, subtitle: Option<&str>) -> egui::Response {
    let response = ui
        .horizontal(|ui| {
            ui.heading(title);
            if let Some(sub) = subtitle {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(sub).color(Palette::of(ui).muted));
                });
            }
        })
        .response;
    ui.add_space(6.0);
    response
}

/// Clearly states that a section is not implemented yet, instead of showing
/// an empty or fake UI.
pub fn not_implemented(ui: &mut egui::Ui, title: &str, phase: &str, planned: &[&str]) {
    page_header(ui, title, None);
    section(ui, "STATUS", |ui| {
        ui.label(
            RichText::new(format!("Not implemented yet. Scheduled for {phase}."))
                .color(Palette::of(ui).warning),
        );
        ui.add_space(4.0);
        ui.label("Planned functionality:");
        for item in planned {
            ui.label(format!("  • {item}"));
        }
    });
}
