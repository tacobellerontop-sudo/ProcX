//! Settings editor. Edits a draft copy; nothing changes until "Save".

use std::path::Path;

use egui::{ComboBox, DragValue, RichText};

use crate::core::config::{
    Config, LogLevel, MAX_CHART_WINDOW_SECS, MAX_HISTORY_SAMPLE_SECS, MAX_REFRESH_MS, MAX_RETENTION_DAYS,
    MIN_CHART_WINDOW_SECS, MIN_HISTORY_SAMPLE_SECS, MIN_REFRESH_MS, MIN_RETENTION_DAYS, ThemePreference,
};
use crate::core::models::Section;
use crate::ui::components::dialogs::{Confirm, ConfirmResult};
use crate::ui::components::{page_header, section};
use crate::ui::theme::Palette;
use crate::utils::paths;

pub enum SettingsAction {
    Save(Config),
}

pub struct SettingsPage {
    draft: Config,
    confirm_reset: bool,
    /// Last save result: (message, is_error).
    pub status: Option<(String, bool)>,
}

impl SettingsPage {
    pub fn new(current: &Config) -> Self {
        Self {
            draft: current.clone(),
            confirm_reset: false,
            status: None,
        }
    }

    /// Updates the draft after settings were changed elsewhere (e.g. the
    /// process table's column chooser), unless the user has unsaved edits.
    pub fn sync(&mut self, saved: &Config) {
        self.draft = saved.clone();
    }

    fn unimplemented_note(ui: &mut egui::Ui, reason: &str) {
        ui.label(
            RichText::new(format!("Saved, but not applied yet ({reason})"))
                .small()
                .color(Palette::of(ui).warning),
        );
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        current: &Config,
        config_path: &Path,
    ) -> Option<SettingsAction> {
        let mut action = None;
        page_header(ui, "Settings", None);

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let d = &mut self.draft;

            section(ui, "APPEARANCE AND STARTUP", |ui| {
                egui::Grid::new("set_general")
                    .num_columns(2)
                    .spacing([24.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Theme");
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut d.general.theme, ThemePreference::Dark, "Dark");
                            ui.selectable_value(&mut d.general.theme, ThemePreference::Light, "Light");
                        });
                        ui.end_row();

                        ui.label("Start on");
                        ComboBox::from_id_salt("start_section")
                            .selected_text(d.general.start_section.label())
                            .show_ui(ui, |ui| {
                                for s in Section::ALL {
                                    ui.selectable_value(&mut d.general.start_section, s, s.label());
                                }
                            });
                        ui.end_row();

                        ui.label("Start minimized");
                        ui.checkbox(&mut d.general.start_minimized, "");
                        ui.end_row();

                        ui.label("Minimize to tray");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut d.general.minimize_to_tray, "");
                            Self::unimplemented_note(ui, "the system tray icon is not implemented");
                        });
                        ui.end_row();
                    });
            });
            ui.add_space(6.0);

            section(ui, "MONITORING", |ui| {
                egui::Grid::new("set_mon")
                    .num_columns(2)
                    .spacing([24.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Refresh interval").on_hover_text(
                            "How often live data is sampled. Shorter intervals cost more CPU.",
                        );
                        ui.add(
                            DragValue::new(&mut d.monitoring.refresh_interval_ms)
                                .range(MIN_REFRESH_MS..=MAX_REFRESH_MS)
                                .speed(50)
                                .suffix(" ms"),
                        );
                        ui.end_row();

                        ui.label("Chart time span");
                        ui.add(
                            DragValue::new(&mut d.monitoring.chart_window_secs)
                                .range(MIN_CHART_WINDOW_SECS..=MAX_CHART_WINDOW_SECS)
                                .speed(5)
                                .suffix(" s"),
                        );
                        ui.end_row();
                    });
            });
            ui.add_space(6.0);

            section(ui, "HISTORY", |ui| {
                ui.label(
                    egui::RichText::new(
                        "One sample per interval: system averages plus the busiest processes. \
                         Older data is deleted after the retention period.",
                    )
                    .small()
                    .color(Palette::of(ui).muted),
                );
                egui::Grid::new("set_hist")
                    .num_columns(2)
                    .spacing([24.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Record history");
                        ui.checkbox(&mut d.history.enabled, "");
                        ui.end_row();

                        ui.label("Retention");
                        ui.add(
                            DragValue::new(&mut d.history.retention_days)
                                .range(MIN_RETENTION_DAYS..=MAX_RETENTION_DAYS)
                                .suffix(" days"),
                        );
                        ui.end_row();

                        ui.label("Sample interval");
                        ui.add(
                            DragValue::new(&mut d.history.sample_interval_secs)
                                .range(MIN_HISTORY_SAMPLE_SECS..=MAX_HISTORY_SAMPLE_SECS)
                                .suffix(" s"),
                        );
                        ui.end_row();

                        ui.label("Database location");
                        let default_db = paths::default_database_file();
                        let mut text = d
                            .history
                            .database_path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut text)
                                .hint_text(default_db.display().to_string())
                                .desired_width(360.0),
                        );
                        if edit.changed() {
                            d.history.database_path = (!text.trim().is_empty()).then(|| text.trim().into());
                        }
                        ui.end_row();
                    });
            });
            ui.add_space(6.0);

            section(ui, "CONFIRMATIONS", |ui| {
                ui.checkbox(
                    &mut d.confirmations.end_process,
                    "Confirm before ending a process",
                );
                ui.checkbox(
                    &mut d.confirmations.service_changes,
                    "Confirm before changing a service",
                );
                ui.checkbox(
                    &mut d.confirmations.startup_changes,
                    "Confirm before changing a startup entry",
                );
            });
            ui.add_space(6.0);

            section(ui, "LOGGING", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Log level");
                    ComboBox::from_id_salt("log_level")
                        .selected_text(d.logging.level.label())
                        .show_ui(ui, |ui| {
                            for level in LogLevel::ALL {
                                ui.selectable_value(&mut d.logging.level, level, level.label());
                            }
                        });
                    ui.label(
                        RichText::new("Applies after restart")
                            .small()
                            .color(Palette::of(ui).muted),
                    );
                });
                ui.label(
                    RichText::new(format!("Log file: {}", paths::log_file().display()))
                        .small()
                        .color(Palette::of(ui).muted),
                );
            });
            ui.add_space(10.0);

            let dirty = self.draft != *current;
            ui.horizontal(|ui| {
                if ui.add_enabled(dirty, egui::Button::new("Save")).clicked() {
                    let mut config = self.draft.clone();
                    let warnings = config.validate();
                    if !warnings.is_empty() {
                        self.status = Some((warnings.join("; "), true));
                    }
                    self.draft = config.clone();
                    action = Some(SettingsAction::Save(config));
                }
                if ui
                    .add_enabled(dirty, egui::Button::new("Discard changes"))
                    .clicked()
                {
                    self.draft = current.clone();
                    self.status = None;
                }
                if ui.button("Reset to defaults…").clicked() {
                    self.confirm_reset = true;
                }
            });
            if let Some((msg, is_error)) = &self.status {
                let p = Palette::of(ui);
                ui.label(RichText::new(msg).color(if *is_error { p.critical } else { p.ok }));
            }
            ui.label(
                RichText::new(format!("Configuration file: {}", config_path.display()))
                    .small()
                    .color(Palette::of(ui).muted),
            );
        });

        if self.confirm_reset {
            let dialog = Confirm {
                id: "confirm_reset_settings",
                title: "Reset settings?".into(),
                message: "All settings will be replaced with their defaults and saved immediately.".into(),
                confirm_label: "Reset".into(),
                destructive: true,
            };
            match dialog.show(ui.ctx()) {
                ConfirmResult::Pending => {}
                ConfirmResult::Cancelled => self.confirm_reset = false,
                ConfirmResult::Confirmed => {
                    self.confirm_reset = false;
                    self.draft = Config::default();
                    action = Some(SettingsAction::Save(Config::default()));
                }
            }
        }
        action
    }
}
