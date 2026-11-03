//! Small metric widgets.

use egui::{Color32, RichText};

use crate::core::health::Severity;
use crate::ui::theme::Palette;
use crate::utils::formatting::UNAVAILABLE;

/// Large value with a caption, used at the top of metric panels.
pub fn headline(ui: &mut egui::Ui, value: &str, caption: &str, color: Color32) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label(RichText::new(value).size(22.0).strong().color(color));
        ui.label(RichText::new(caption).color(Palette::of(ui).muted));
    })
    .response
}

/// A labelled value cell with a tooltip explaining the metric.
/// `None` is shown as "unavailable", never as zero.
pub fn stat(ui: &mut egui::Ui, label: &str, value: Option<String>, tooltip: &str) {
    let palette = Palette::of(ui);
    ui.vertical(|ui| {
        ui.label(RichText::new(label).small().color(palette.muted));
        let response = match value {
            Some(v) => ui.label(RichText::new(v).monospace().strong()),
            None => ui.label(RichText::new(UNAVAILABLE).monospace().color(palette.muted)),
        };
        response.on_hover_text(tooltip);
    })
    .response
    .on_hover_text(tooltip);
}

/// Colored dot followed by text.
pub fn status_dot(ui: &mut egui::Ui, color: Color32, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, color);
        ui.label(text);
    })
    .response
}

pub fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Ok => "OK",
        Severity::Warning => "Warning",
        Severity::Critical => "Critical",
    }
}

/// Thin horizontal usage bar (0..=100).
pub fn usage_bar(ui: &mut egui::Ui, percent: f32, color: Color32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 4.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 1.0, Palette::of(ui).border);
    let mut fill = rect;
    fill.set_width(rect.width() * (percent / 100.0).clamp(0.0, 1.0));
    painter.rect_filled(fill, 1.0, color);
}
