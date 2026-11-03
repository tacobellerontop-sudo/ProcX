//! Modal dialogs.

use egui::{Id, Modal, RichText};

use crate::ui::theme::Palette;

/// A confirmation dialog for potentially disruptive actions.
#[derive(Debug, Clone)]
pub struct Confirm {
    pub id: &'static str,
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    /// Styles the confirm button as destructive.
    pub destructive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmResult {
    Pending,
    Confirmed,
    Cancelled,
}

impl Confirm {
    pub fn show(&self, ctx: &egui::Context) -> ConfirmResult {
        let mut result = ConfirmResult::Pending;
        let modal = Modal::new(Id::unique(self.id)).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading(&self.title);
            ui.add_space(6.0);
            ui.label(&self.message);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let palette = Palette::of(ui);
                let label = RichText::new(&self.confirm_label);
                let label = if self.destructive {
                    label.color(palette.critical)
                } else {
                    label
                };
                if ui.button(label).clicked() {
                    result = ConfirmResult::Confirmed;
                }
                if ui.button("Cancel").clicked() {
                    result = ConfirmResult::Cancelled;
                }
            });
        });
        if result == ConfirmResult::Pending && modal.should_close() {
            result = ConfirmResult::Cancelled;
        }
        result
    }
}
