//! Top-level application: owns state, workers and the frame layout.

pub mod live;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use egui::RichText;

use crate::core::config::{Config, LoadedConfig};
use crate::core::events::CollectorCommand;
use crate::core::models::Section;
use crate::monitoring::collector::Collector;
use crate::ui::components::metrics::status_dot;
use crate::ui::components::sidebar;
use crate::ui::navigation::{self, GlobalAction};
use crate::ui::pages::{self, settings::SettingsAction, settings::SettingsPage};
use crate::ui::theme::{self, Palette};
use live::LiveState;

pub struct ProcxApp {
    config: Config,
    config_path: PathBuf,
    collector: Option<Collector>,
    live: LiveState,
    section: Section,
    settings: SettingsPage,
    first_frame: bool,
}

impl ProcxApp {
    pub fn new(cc: &eframe::CreationContext<'_>, loaded: LoadedConfig, config_path: PathBuf) -> Self {
        let config = loaded.config;
        theme::install(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, config.general.theme);

        let mut live = LiveState::new(
            config.monitoring.chart_window_secs,
            config.monitoring.refresh_interval_ms,
        );
        live.warnings.extend(loaded.warnings);

        let ctx = cc.egui_ctx.clone();
        let collector = match Collector::spawn(
            Duration::from_millis(config.monitoring.refresh_interval_ms),
            Arc::new(move || ctx.request_repaint()),
        ) {
            Ok(c) => Some(c),
            Err(err) => {
                tracing::error!("{err}");
                live.warnings.push(err.to_string());
                None
            }
        };

        Self {
            section: config.general.start_section,
            settings: SettingsPage::new(&config),
            config,
            config_path,
            collector,
            live,
            first_frame: true,
        }
    }

    fn refresh_interval(&self) -> Duration {
        Duration::from_millis(self.config.monitoring.refresh_interval_ms)
    }

    fn apply_settings(&mut self, ctx: &egui::Context, new: Config) {
        match new.save(&self.config_path) {
            Ok(()) => {
                tracing::info!("settings saved to {}", self.config_path.display());
                self.settings.status = Some(("Settings saved".into(), false));
            }
            Err(err) => {
                tracing::error!("{err}");
                self.settings.status = Some((format!("Could not save settings: {err}"), true));
                return;
            }
        }
        if new.general.theme != self.config.general.theme {
            theme::apply(ctx, new.general.theme);
        }
        if new.monitoring != self.config.monitoring {
            self.live.resize(
                new.monitoring.chart_window_secs,
                new.monitoring.refresh_interval_ms,
            );
            if let Some(c) = &self.collector {
                c.send(CollectorCommand::SetInterval(Duration::from_millis(
                    new.monitoring.refresh_interval_ms,
                )));
            }
        }
        self.config = new;
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        ui.horizontal(|ui| {
            let alive = self.collector.as_ref().is_some_and(Collector::is_alive);
            let (color, text) = if !alive {
                (p.critical, "Monitoring stopped")
            } else if self.live.is_stale(self.refresh_interval()) {
                (p.warning, "Waiting for data")
            } else {
                (p.ok, "Live")
            };
            status_dot(ui, color, RichText::new(text).small());
            ui.separator();
            if let Some(s) = &self.live.latest {
                ui.label(
                    RichText::new(format!(
                        "Interval {} ms · sample took {:.1} ms",
                        self.config.monitoring.refresh_interval_ms,
                        s.collection_time.as_secs_f64() * 1000.0
                    ))
                    .small()
                    .color(p.muted),
                )
                .on_hover_text("Time the background collector spent gathering the last sample");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(info) = &self.live.info {
                    let privilege = match info.elevated {
                        Some(true) => "Administrator",
                        Some(false) => "Standard user",
                        None => "Privilege unknown",
                    };
                    ui.label(RichText::new(privilege).small().color(p.muted))
                        .on_hover_text(
                            "ProcX runs with least privilege. Some details of system and other users' \
                         processes need administrator rights and are shown as unavailable.",
                        );
                }
                if !self.live.warnings.is_empty() {
                    ui.separator();
                    ui.label(
                        RichText::new(format!("{} warning(s)", self.live.warnings.len()))
                            .small()
                            .color(p.warning),
                    )
                    .on_hover_text(self.live.warnings.join("\n"));
                }
            });
        });
    }
}

impl eframe::App for ProcxApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.first_frame {
            self.first_frame = false;
            if self.config.general.start_minimized {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }

        if let Some(collector) = &self.collector {
            for event in collector.drain() {
                self.live.ingest(event);
            }
        }

        match navigation::handle_shortcuts(&ctx) {
            Some(GlobalAction::Navigate(s)) => self.section = s,
            Some(GlobalAction::Refresh) => {
                if let Some(c) = &self.collector {
                    c.send(CollectorCommand::RefreshNow);
                }
            }
            None => {}
        }

        egui::Panel::bottom("status_bar")
            .exact_size(24.0)
            .show(ui, |ui| self.status_bar(ui));

        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(168.0)
            .size_range(140.0..=260.0)
            .show(ui, |ui| {
                if let Some(s) = sidebar::show(ui, self.section) {
                    self.section = s;
                }
            });

        let mut settings_action = None;
        egui::CentralPanel::default_margins().show(ui, |ui| match self.section {
            Section::Overview => {
                pages::overview::show(ui, &self.live, self.config.monitoring.chart_window_secs)
            }
            Section::Processes => pages::processes::show(ui),
            Section::Performance => pages::performance::show(ui),
            Section::Services => pages::services::show(ui),
            Section::Startup => pages::startup::show(ui),
            Section::Details => pages::details::show(ui),
            Section::History => pages::history::show(ui),
            Section::Settings => {
                settings_action = self.settings.show(ui, &self.config, &self.config_path);
            }
        });
        if let Some(SettingsAction::Save(new)) = settings_action {
            self.apply_settings(&ctx, new);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(mut collector) = self.collector.take() {
            collector.shutdown();
        }
        tracing::info!("ProcX exiting");
    }
}
