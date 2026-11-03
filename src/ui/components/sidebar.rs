//! Persistent navigation rail.

use egui::{RichText, Sense, Stroke};

use crate::core::models::Section;
use crate::ui::navigation::section_shortcut;
use crate::ui::pages::{self, PageStatus};
use crate::ui::theme::Palette;

/// Draws the navigation list. Returns the newly selected section, if any.
pub fn show(ui: &mut egui::Ui, current: Section) -> Option<Section> {
    let palette = Palette::of(ui);
    let mut selected = None;

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(RichText::new("ProcX").size(18.0).strong().color(palette.accent));
    });
    ui.add_space(12.0);

    for section in Section::ALL {
        let status = pages::status(section);
        let is_current = section == current;
        let row_height = 26.0;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), row_height), Sense::click());

        let visuals = ui.visuals();
        if is_current {
            ui.painter().rect_filled(rect, 2.0, visuals.selection.bg_fill);
            ui.painter().line_segment(
                [rect.left_top(), rect.left_bottom()],
                Stroke::new(2.0, palette.accent),
            );
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, 2.0, visuals.widgets.hovered.weak_bg_fill);
        }

        let text_color = match (is_current, status) {
            (true, _) => visuals.strong_text_color(),
            (false, PageStatus::Ready) => visuals.text_color(),
            (false, PageStatus::Planned(_)) => palette.muted,
        };
        ui.painter().text(
            rect.left_center() + egui::vec2(12.0, 0.0),
            egui::Align2::LEFT_CENTER,
            section.label(),
            egui::TextStyle::Button.resolve(ui.style()),
            text_color,
        );

        let shortcut = ui.ctx().format_shortcut(&section_shortcut(section));
        let tip = match status {
            PageStatus::Ready => shortcut,
            PageStatus::Planned(phase) => format!("{shortcut} · not implemented yet ({phase})"),
        };
        if response.on_hover_text(tip).clicked() {
            selected = Some(section);
        }
    }
    selected
}
