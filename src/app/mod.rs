//! Top-level application: owns state, workers and the frame layout.

pub mod live;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::RichText;

use crate::core::config::{Config, LoadedConfig};
use crate::core::events::CollectorCommand;
use crate::core::models::Section;
use crate::history::worker::{HistoryRequest, HistoryResponse, HistoryWorker};
use crate::monitoring::collector::{Collector, Notifier};
use crate::process::worker::{ProcessRequest, ProcessResponse, ProcessWorker};
use crate::ui::components::metrics::status_dot;
use crate::ui::components::sidebar;
use crate::ui::navigation::{self, GlobalAction};
use crate::ui::pages::details::{DetailsAction, DetailsPage};
use crate::ui::pages::history::{HistoryAction, HistoryPage};
use crate::ui::pages::performance::PerformancePage;
use crate::ui::pages::processes::{ProcessesAction, ProcessesPage};
use crate::ui::pages::services::{ServicesAction, ServicesPage};
use crate::ui::pages::startup::{StartupAction, StartupPage};
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
    processes_page: ProcessesPage,
    details_page: DetailsPage,
    performance_page: PerformancePage,
    services_page: ServicesPage,
    startup_page: StartupPage,
    history_page: HistoryPage,
    history: Option<HistoryWorker>,
    worker: Option<ProcessWorker>,
    /// Transient status message: (text, is_error, shown_at).
    notice: Option<(String, bool, Instant)>,
    own_pid: u32,
    first_frame: bool,
    show_help: bool,
    confirm_elevate: bool,
}

const NOTICE_DURATION: Duration = Duration::from_secs(8);

impl ProcxApp {
    pub fn new(cc: &eframe::CreationContext<'_>, loaded: LoadedConfig, config_path: PathBuf) -> Self {
        let config = loaded.config;
        theme::install(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, config.general.theme);

        let mut live = LiveState::new(config.monitoring.refresh_interval_ms);
        live.warnings.extend(loaded.warnings);

        let ctx = cc.egui_ctx.clone();
        let notify: Notifier = Arc::new(move || ctx.request_repaint());
        let worker = match ProcessWorker::spawn(notify.clone()) {
            Ok(w) => Some(w),
            Err(err) => {
                tracing::error!("{err}");
                live.warnings.push(err.to_string());
                None
            }
        };
        let history = match HistoryWorker::spawn(config.history.clone(), notify.clone()) {
            Ok(h) => Some(h),
            Err(err) => {
                tracing::error!("{err}");
                live.warnings.push(err.to_string());
                None
            }
        };
        let collector = match Collector::spawn(
            Duration::from_millis(config.monitoring.refresh_interval_ms),
            notify,
            history.as_ref().map(HistoryWorker::tick_sender),
        ) {
            Ok(c) => Some(c),
            Err(err) => {
                tracing::error!("{err}");
                live.warnings.push(err.to_string());
                None
            }
        };

        let config_chart_window = config.monitoring.chart_window_secs;
        Self {
            section: config.general.start_section,
            settings: SettingsPage::new(&config),
            config,
            config_path,
            collector,
            live,
            processes_page: ProcessesPage::default(),
            details_page: DetailsPage::default(),
            performance_page: PerformancePage::new(config_chart_window),
            services_page: ServicesPage::default(),
            startup_page: StartupPage::default(),
            history_page: HistoryPage::default(),
            history,
            worker,
            notice: None,
            own_pid: std::process::id(),
            first_frame: true,
            show_help: false,
            confirm_elevate: false,
        }
    }

    fn notify(&mut self, text: impl Into<String>, is_error: bool) {
        self.notice = Some((text.into(), is_error, Instant::now()));
    }

    fn send_worker(&mut self, request: ProcessRequest) {
        match &self.worker {
            Some(w) => w.send(request),
            None => self.notify("The process worker is not running", true),
        }
    }

    fn refresh_now(&self) {
        if let Some(c) = &self.collector {
            c.send(CollectorCommand::RefreshNow);
        }
    }

    fn show_details(&mut self, key: crate::core::models::ProcessKey) {
        let processes = self.live.latest.as_ref().map(|s| &*s.processes);
        if self.details_page.set_target(key, processes) {
            self.send_worker(ProcessRequest::Inspect(key));
        }
        self.section = Section::Details;
    }

    fn handle_worker_responses(&mut self, ctx: &egui::Context) {
        let responses: Vec<ProcessResponse> = match &self.worker {
            Some(w) => w.drain().collect(),
            None => return,
        };
        for response in responses {
            match response {
                ProcessResponse::Details(key, details) => self.details_page.on_details(key, *details),
                ProcessResponse::Terminated { key, name, result } => match result {
                    Ok(()) => {
                        self.notify(format!("Ended {name} (PID {})", key.pid), false);
                        self.refresh_now();
                    }
                    Err(e) => self.notify(format!("Could not end {name} (PID {}): {e}", key.pid), true),
                },
                ProcessResponse::OpenLocationFailed(e) => {
                    self.notify(format!("Could not open file location: {e}"), true)
                }
                ProcessResponse::Services(result) => self.services_page.on_services(result),
                ProcessResponse::Relaunched(Ok(())) => {
                    tracing::info!("elevated instance started; closing this one");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                ProcessResponse::Relaunched(Err(e)) => self.notify(e, true),
                ProcessResponse::Startup(result) => self.startup_page.on_entries(result),
                ProcessResponse::ServiceChanged { action, result }
                | ProcessResponse::StartupChanged { action, result } => match result {
                    Ok(()) => self.notify(format!("{action}: done"), false),
                    Err(e) => self.notify(format!("{action} failed: {e}"), true),
                },
            }
        }
    }

    fn send_history(&mut self, request: HistoryRequest) {
        match &self.history {
            Some(h) => h.send(request),
            None => self.notify("History recording is not running", true),
        }
    }

    fn handle_history_responses(&mut self) {
        let responses: Vec<HistoryResponse> = match &self.history {
            Some(h) => h.drain().collect(),
            None => return,
        };
        for response in responses {
            match response {
                HistoryResponse::Status(status) => self.history_page.on_status(status),
                HistoryResponse::Range {
                    from_ms,
                    to_ms,
                    system,
                    processes,
                } => self.history_page.on_range(from_ms, to_ms, system, processes),
                HistoryResponse::ProcessSeries { id, points } => self.history_page.on_series(id, points),
                HistoryResponse::Exported(result) => {
                    match &result {
                        Ok(path) => self.notify(format!("Exported {}", path.display()), false),
                        Err(e) => self.notify(format!("Export failed: {e}"), true),
                    }
                    self.history_page.on_exported(result);
                }
            }
        }
    }

    fn handle_history_actions(&mut self, actions: Vec<HistoryAction>) {
        for action in actions {
            match action {
                HistoryAction::Query { from_ms, to_ms } => {
                    self.send_history(HistoryRequest::Range { from_ms, to_ms })
                }
                HistoryAction::ProcessSeries { id, from_ms, to_ms } => {
                    self.send_history(HistoryRequest::ProcessSeries { id, from_ms, to_ms })
                }
                HistoryAction::ExportSystem { from_ms, to_ms } => {
                    self.send_history(HistoryRequest::ExportSystem { from_ms, to_ms })
                }
                HistoryAction::ExportProcess { row, from_ms, to_ms } => {
                    self.send_history(HistoryRequest::ExportProcess { row, from_ms, to_ms })
                }
                HistoryAction::ShowFile(path) => self.send_worker(ProcessRequest::OpenLocation(path)),
                HistoryAction::OpenSettings => self.navigate(Section::Settings),
            }
        }
    }

    fn handle_processes_actions(&mut self, ctx: &egui::Context, actions: Vec<ProcessesAction>) {
        for action in actions {
            match action {
                ProcessesAction::Terminate(targets) => self.send_worker(ProcessRequest::Terminate(targets)),
                ProcessesAction::OpenLocation(path) => self.send_worker(ProcessRequest::OpenLocation(path)),
                ProcessesAction::ShowDetails(key) => self.show_details(key),
                ProcessesAction::Refresh => self.refresh_now(),
                ProcessesAction::Notice(text) => self.notify(text, false),
                ProcessesAction::SaveConfig(processes) => {
                    let mut new = self.config.clone();
                    new.processes = processes;
                    self.apply_settings(ctx, new);
                }
            }
        }
    }

    fn handle_details_actions(&mut self, actions: Vec<DetailsAction>) {
        for action in actions {
            match action {
                DetailsAction::Inspect(key) => self.send_worker(ProcessRequest::Inspect(key)),
                DetailsAction::OpenLocation(path) => self.send_worker(ProcessRequest::OpenLocation(path)),
                DetailsAction::Select(key) => {
                    self.processes_page.select(key);
                    self.show_details(key);
                }
            }
        }
    }

    fn handle_services_actions(&mut self, actions: Vec<ServicesAction>) {
        for action in actions {
            match action {
                ServicesAction::Refresh => {
                    self.services_page.set_loading();
                    self.send_worker(ProcessRequest::ListServices);
                }
                ServicesAction::Control { name, control } => {
                    self.services_page.set_loading();
                    self.send_worker(ProcessRequest::ControlService { name, control });
                }
                ServicesAction::SetStartType { name, start } => {
                    self.services_page.set_loading();
                    self.send_worker(ProcessRequest::SetServiceStartType { name, start });
                }
                ServicesAction::ShowProcess(pid) => {
                    let key = self
                        .live
                        .latest
                        .as_ref()
                        .and_then(|s| s.processes.iter().find(|p| p.key.pid == pid).map(|p| p.key));
                    match key {
                        Some(key) => {
                            self.processes_page.select(key);
                            self.show_details(key);
                        }
                        None => {
                            self.notify(format!("Process {pid} is not in the current process list"), true)
                        }
                    }
                }
            }
        }
    }

    fn handle_startup_actions(&mut self, ctx: &egui::Context, actions: Vec<StartupAction>) {
        for action in actions {
            match action {
                StartupAction::Refresh => {
                    self.startup_page.set_loading();
                    self.send_worker(ProcessRequest::ListStartup);
                }
                StartupAction::SetStatus { source, name, status } => {
                    self.startup_page.set_loading();
                    self.send_worker(ProcessRequest::SetStartupStatus { source, name, status });
                }
                StartupAction::OpenLocation(path) => self.send_worker(ProcessRequest::OpenLocation(path)),
                StartupAction::Copy(text) => {
                    ctx.copy_text(text);
                    self.notify("Copied to clipboard", false);
                }
            }
        }
    }

    /// Switches sections. Opening Details without a target inspects the
    /// process selected on the Processes page.
    fn navigate(&mut self, section: Section) {
        if section == Section::Details
            && self.details_page.target().is_none()
            && let Some(key) = self.processes_page.primary_selection()
        {
            self.show_details(key);
            return;
        }
        self.section = section;
    }

    fn refresh_interval(&self) -> Duration {
        Duration::from_millis(self.config.monitoring.refresh_interval_ms)
    }

    fn apply_settings(&mut self, ctx: &egui::Context, new: Config) {
        match new.save(&self.config_path) {
            Ok(()) => {
                tracing::info!("settings saved to {}", self.config_path.display());
                self.settings.status = Some(("Settings saved".into(), false));
                self.settings.sync(&new);
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
        if new.history != self.config.history
            && let Some(h) = &self.history
        {
            h.send(HistoryRequest::Configure(new.history.clone()));
        }
        if new.monitoring != self.config.monitoring {
            self.live.resize(new.monitoring.refresh_interval_ms);
            if let Some(c) = &self.collector {
                c.send(CollectorCommand::SetInterval(Duration::from_millis(
                    new.monitoring.refresh_interval_ms,
                )));
            }
        }
        self.config = new;
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
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
            let history = match (&self.history, self.history_page.status()) {
                (None, _) => Some((
                    p.critical,
                    "History stopped",
                    "The history thread is not running".to_owned(),
                )),
                (Some(_), Some(s)) if s.error.is_some() => {
                    Some((p.critical, "History error", s.error.clone().unwrap_or_default()))
                }
                (Some(_), Some(s)) if s.enabled => Some((
                    p.ok,
                    "Recording",
                    format!(
                        "Recording history every {} s to {}",
                        self.config.history.sample_interval_secs,
                        s.path.display()
                    ),
                )),
                (Some(_), Some(_)) => Some((p.muted, "History off", "History recording is off".to_owned())),
                (Some(_), None) => None,
            };
            if let Some((color, text, tip)) = history {
                ui.separator();
                status_dot(ui, color, RichText::new(text).small()).on_hover_text(tip);
            }
            if let Some((text, is_error, at)) = &self.notice
                && at.elapsed() < NOTICE_DURATION
            {
                ui.separator();
                ui.label(
                    RichText::new(text)
                        .small()
                        .color(if *is_error { p.critical } else { p.ok }),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("?")
                    .on_hover_text("Keyboard shortcuts (F1)")
                    .clicked()
                {
                    self.show_help = true;
                }
                if let Some(info) = &self.live.info {
                    let privilege = match info.elevated {
                        Some(true) => "Administrator",
                        Some(false) => "Standard user",
                        None => "Privilege unknown",
                    };
                    if info.elevated == Some(false)
                        && ui
                            .small_button("Restart as administrator…")
                            .on_hover_text(
                                "Needed to manage services and all-users startup entries, and to see details \
                                 of protected processes. Windows asks for permission first.",
                            )
                            .clicked()
                    {
                        self.confirm_elevate = true;
                    }
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

impl ProcxApp {
    fn dialogs(&mut self, ctx: &egui::Context) {
        use crate::ui::components::dialogs::{Confirm, ConfirmResult};
        if self.confirm_elevate {
            let dialog = Confirm {
                id: "confirm_elevate",
                title: "Restart as administrator?".into(),
                message: "Windows will ask for permission to start ProcX with administrator rights. If you \
                          allow it, this window closes and the new one opens. Monitoring works without \
                          administrator rights; they are needed only to manage services, change all-users \
                          startup entries and inspect protected processes."
                    .into(),
                confirm_label: "Restart".into(),
                destructive: false,
            };
            match dialog.show(ctx) {
                ConfirmResult::Pending => {}
                ConfirmResult::Cancelled => self.confirm_elevate = false,
                ConfirmResult::Confirmed => {
                    self.confirm_elevate = false;
                    self.send_worker(ProcessRequest::RelaunchElevated);
                }
            }
        }
        if self.show_help {
            let shortcuts = navigation::all_shortcuts(ctx);
            let modal = egui::Modal::new(egui::Id::unique("shortcuts_help")).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.heading("Keyboard shortcuts");
                ui.add_space(6.0);
                egui::Grid::new("shortcuts_grid")
                    .num_columns(2)
                    .spacing([18.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for (keys, what) in &shortcuts {
                            ui.label(RichText::new(keys).monospace().strong());
                            ui.label(what);
                            ui.end_row();
                        }
                    });
                ui.add_space(8.0);
                ui.button("Close").clicked()
            });
            if modal.inner || modal.should_close() {
                self.show_help = false;
            }
        }
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

        let mut new_snapshot = false;
        if let Some(collector) = &self.collector {
            for event in collector.drain() {
                new_snapshot |= self.live.ingest(event);
            }
        }
        if new_snapshot && let Some(s) = &self.live.latest {
            self.details_page.on_snapshot(&s.processes, self.live.chart_now());
        }
        self.handle_worker_responses(&ctx);
        self.handle_history_responses();
        if let Some((_, _, at)) = &self.notice {
            // Repaint once more so the notice disappears on time.
            ctx.request_repaint_after(NOTICE_DURATION.saturating_sub(at.elapsed()));
        }

        match navigation::handle_shortcuts(&ctx) {
            Some(GlobalAction::Navigate(s)) => self.navigate(s),
            Some(GlobalAction::Refresh) => self.refresh_now(),
            Some(GlobalAction::Help) => self.show_help = true,
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
                    self.navigate(s);
                }
            });

        let mut settings_action = None;
        let mut process_actions = Vec::new();
        let mut details_actions = Vec::new();
        let mut services_actions = Vec::new();
        let mut startup_actions = Vec::new();
        let mut history_actions = Vec::new();
        let elevated = self.live.info.as_ref().and_then(|i| i.elevated);
        let total_memory = self.live.latest.as_ref().map_or(0, |s| s.memory.total);
        let window = f64::from(self.config.monitoring.chart_window_secs);
        egui::CentralPanel::default_margins().show(ui, |ui| match self.section {
            Section::Overview => {
                pages::overview::show(ui, &self.live, self.config.monitoring.chart_window_secs)
            }
            Section::Processes => {
                process_actions = self.processes_page.show(
                    ui,
                    self.live.latest.as_ref().map(|s| &s.processes),
                    total_memory,
                    &self.config.processes,
                    self.config.confirmations.end_process,
                    self.own_pid,
                );
            }
            Section::Performance => {
                self.performance_page
                    .show(ui, &self.live, self.config.monitoring.refresh_interval_ms)
            }
            Section::Services => {
                services_actions =
                    self.services_page
                        .show(ui, self.config.confirmations.service_changes, elevated);
            }
            Section::Startup => {
                startup_actions =
                    self.startup_page
                        .show(ui, self.config.confirmations.startup_changes, elevated);
            }
            Section::Details => {
                details_actions = self.details_page.show(
                    ui,
                    self.live.latest.as_ref().map(|s| &*s.processes),
                    total_memory,
                    self.live.chart_now(),
                    window,
                );
            }
            Section::History => {
                history_actions = self
                    .history_page
                    .show(ui, self.config.history.sample_interval_secs);
            }
            Section::Settings => {
                settings_action = self.settings.show(ui, &self.config, &self.config_path);
            }
        });
        self.handle_processes_actions(&ctx, process_actions);
        self.handle_details_actions(details_actions);
        self.handle_services_actions(services_actions);
        self.handle_startup_actions(&ctx, startup_actions);
        self.handle_history_actions(history_actions);
        self.dialogs(&ctx);
        if let Some(SettingsAction::Save(new)) = settings_action {
            self.apply_settings(&ctx, new);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(mut collector) = self.collector.take() {
            collector.shutdown();
        }
        // After the collector, so the last partial interval is written.
        if let Some(mut history) = self.history.take() {
            history.shutdown();
        }
        if let Some(mut worker) = self.worker.take() {
            worker.shutdown();
        }
        tracing::info!("ProcX exiting");
    }
}
