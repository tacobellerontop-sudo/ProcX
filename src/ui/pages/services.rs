//! Windows services: list, search, properties, and confirmed start/stop and
//! start-type changes.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use egui::RichText;

use crate::core::services::{self, ServiceControl, ServiceInfo, ServiceState, StartType};
use crate::ui::components::dialogs::{Confirm, ConfirmResult};
use crate::ui::components::list_table::{self, ListColumn, ListSort, ListTable};
use crate::ui::components::tables::key_values;
use crate::ui::components::{page_header, section};
use crate::ui::theme::Palette;
use crate::utils::formatting::UNAVAILABLE;

#[derive(Debug, Clone, PartialEq)]
pub enum ServicesAction {
    Refresh,
    Control {
        name: String,
        control: ServiceControl,
    },
    SetStartType {
        name: String,
        start: StartType,
    },
    /// Show the hosting process on the Details page.
    ShowProcess(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateFilter {
    All,
    Running,
    Stopped,
}

impl StateFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Running => "Running",
            Self::Stopped => "Stopped",
        }
    }

    fn accepts(self, state: ServiceState) -> bool {
        match self {
            Self::All => true,
            Self::Running => state.is_running(),
            Self::Stopped => !state.is_running(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Pending {
    Control(ServiceControl),
    StartType(StartType),
}

/// How often the list refreshes while a service is starting or stopping.
const PENDING_REFRESH: Duration = Duration::from_secs(2);

const COLUMNS: [ListColumn; 6] = [
    ListColumn {
        label: "Name",
        width: 220.0,
        tooltip: "Display name",
    },
    ListColumn {
        label: "Service",
        width: 140.0,
        tooltip: "Service (key) name",
    },
    ListColumn {
        label: "Status",
        width: 80.0,
        tooltip: "",
    },
    ListColumn {
        label: "Startup type",
        width: 130.0,
        tooltip: "",
    },
    ListColumn {
        label: "PID",
        width: 60.0,
        tooltip: "Process ID of the hosting process, when running",
    },
    ListColumn {
        label: "Description",
        width: 200.0,
        tooltip: "",
    },
];

pub struct ServicesPage {
    services: Vec<ServiceInfo>,
    error: Option<String>,
    loaded_at: Option<Instant>,
    loading: bool,
    search: String,
    filter: StateFilter,
    sort: ListSort,
    selected: Option<String>,
    pending: Option<(String, Pending)>,
    /// Indices into `services` after filtering and sorting.
    view: Vec<usize>,
    view_dirty: bool,
}

impl Default for ServicesPage {
    fn default() -> Self {
        Self {
            services: Vec::new(),
            error: None,
            loaded_at: None,
            loading: false,
            search: String::new(),
            filter: StateFilter::All,
            sort: ListSort::default(),
            selected: None,
            pending: None,
            view: Vec::new(),
            view_dirty: true,
        }
    }
}

fn compare(a: &ServiceInfo, b: &ServiceInfo, column: usize) -> Ordering {
    let text = |x: &str, y: &str| x.to_lowercase().cmp(&y.to_lowercase());
    match column {
        0 => text(&a.display_name, &b.display_name),
        1 => text(&a.name, &b.name),
        2 => a.state.label().cmp(&b.state.label()),
        3 => a
            .start_type
            .map(StartType::label)
            .cmp(&b.start_type.map(StartType::label)),
        4 => a.pid.cmp(&b.pid),
        _ => a.description.cmp(&b.description),
    }
}

impl ServicesPage {
    /// Called when the app sends a list request.
    pub fn set_loading(&mut self) {
        self.loading = true;
    }

    pub fn on_services(&mut self, result: Result<Vec<ServiceInfo>, String>) {
        self.loading = false;
        self.loaded_at = Some(Instant::now());
        match result {
            Ok(list) => {
                self.services = list;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
        self.view_dirty = true;
    }

    fn rebuild_view(&mut self) {
        let query = self.search.trim().to_lowercase();
        let mut view: Vec<usize> = self
            .services
            .iter()
            .enumerate()
            .filter(|(_, s)| self.filter.accepts(s.state) && services::matches(s, &query))
            .map(|(i, _)| i)
            .collect();
        let (column, descending) = (self.sort.column, self.sort.descending);
        view.sort_by(|&a, &b| {
            let ord = compare(&self.services[a], &self.services[b], column)
                .then_with(|| self.services[a].name.cmp(&self.services[b].name));
            if descending { ord.reverse() } else { ord }
        });
        self.view = view;
        self.view_dirty = false;
    }

    fn selected_service(&self) -> Option<&ServiceInfo> {
        let name = self.selected.as_deref()?;
        self.services.iter().find(|s| s.name == name)
    }

    pub fn show(&mut self, ui: &mut egui::Ui, confirm: bool, elevated: Option<bool>) -> Vec<ServicesAction> {
        let mut actions = Vec::new();
        let any_pending = self
            .services
            .iter()
            .any(|s| matches!(s.state, ServiceState::StartPending | ServiceState::StopPending));
        let stale = self
            .loaded_at
            .is_none_or(|t| any_pending && t.elapsed() >= PENDING_REFRESH);
        if !self.loading && (self.loaded_at.is_none() || stale) {
            actions.push(ServicesAction::Refresh);
        } else if any_pending {
            ui.ctx().request_repaint_after(PENDING_REFRESH);
        }
        if self.view_dirty {
            self.rebuild_view();
        }

        let palette = Palette::of(ui);
        let running = self.services.iter().filter(|s| s.state.is_running()).count();
        let subtitle = if self.services.is_empty() {
            None
        } else {
            Some(format!("{} services · {running} running", self.services.len()))
        };
        page_header(ui, "Services", subtitle.as_deref());

        ui.horizontal(|ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Search name, description or PID")
                    .desired_width(260.0),
            );
            if search.changed() {
                self.view_dirty = true;
            }
            egui::ComboBox::from_id_salt("service_filter")
                .selected_text(self.filter.label())
                .show_ui(ui, |ui| {
                    for f in [StateFilter::All, StateFilter::Running, StateFilter::Stopped] {
                        if ui.selectable_value(&mut self.filter, f, f.label()).changed() {
                            self.view_dirty = true;
                        }
                    }
                });
            if ui
                .add_enabled(!self.loading, egui::Button::new("Refresh"))
                .on_hover_text("Reload the service list (F5 refreshes live data)")
                .clicked()
            {
                actions.push(ServicesAction::Refresh);
            }
            if self.loading {
                ui.spinner();
            }
            ui.label(
                RichText::new(format!("{} shown", self.view.len()))
                    .small()
                    .color(palette.muted),
            );
        });
        if elevated == Some(false) {
            ui.label(
                RichText::new(
                    "Running as a standard user: Windows usually requires administrator rights to start, stop \
                     or reconfigure services. ProcX never elevates itself.",
                )
                .small()
                .color(palette.muted),
            );
        }
        if let Some(e) = &self.error {
            ui.label(RichText::new(format!("Could not read services: {e}")).color(palette.critical));
        }
        ui.add_space(4.0);

        if self.services.is_empty() {
            if self.error.is_some() {
                return actions;
            }
            ui.label(
                RichText::new(if self.loading {
                    "Loading services…"
                } else {
                    "No services"
                })
                .color(palette.muted),
            );
            return actions;
        }

        let selected_row = self
            .selected
            .as_deref()
            .and_then(|name| self.view.iter().position(|&i| self.services[i].name == name));
        let table_height = (ui.available_height() - 250.0).max(120.0);
        let events = list_table::show(
            ui,
            ListTable {
                id: "services_table",
                columns: &COLUMNS,
                sort: self.sort,
                rows: self.view.len(),
                selected: selected_row,
                height: table_height,
            },
            |ui, row, column| {
                let s = &self.services[self.view[row]];
                match column {
                    0 => list_table::text(ui, Some(&s.display_name)),
                    1 => list_table::text(ui, Some(&s.name)),
                    2 => {
                        let color = match s.state {
                            ServiceState::Running => palette.ok,
                            ServiceState::Stopped => palette.muted,
                            _ => palette.warning,
                        };
                        ui.label(RichText::new(s.state.label()).color(color));
                    }
                    3 => list_table::text(ui, s.start_type.map(StartType::label)),
                    4 => match s.pid {
                        Some(pid) => {
                            ui.label(RichText::new(pid.to_string()).monospace());
                        }
                        None => {
                            ui.label(RichText::new(UNAVAILABLE).color(palette.muted));
                        }
                    },
                    _ => list_table::text(ui, s.description.as_deref()),
                }
            },
        );
        if let Some(c) = events.sort {
            self.sort.toggle(c);
            self.view_dirty = true;
        }
        if let Some(row) = events.clicked.or(events.double_clicked) {
            self.selected = Some(self.services[self.view[row]].name.clone());
        }

        ui.add_space(6.0);
        self.properties(ui, confirm, &mut actions);
        self.dialog(ui.ctx(), &mut actions);
        actions
    }

    fn properties(&mut self, ui: &mut egui::Ui, confirm: bool, actions: &mut Vec<ServicesAction>) {
        let Some(s) = self.selected_service().cloned() else {
            section(ui, "PROPERTIES", |ui| {
                ui.label(
                    RichText::new("Select a service to see its properties").color(Palette::of(ui).muted),
                );
            });
            return;
        };
        let mut request = None;
        egui::ScrollArea::vertical()
            .id_salt("service_props")
            .show(ui, |ui| {
                section(ui, "PROPERTIES", |ui| {
                    ui.horizontal(|ui| {
                        ui.strong(&s.display_name);
                        ui.label(RichText::new(format!("({})", s.name)).color(Palette::of(ui).muted));
                    });
                    ui.horizontal(|ui| {
                        let can_start = !s.state.is_running() && s.start_type != Some(StartType::Disabled);
                        if ui
                            .add_enabled(can_start, egui::Button::new("Start"))
                            .on_disabled_hover_text(
                                "Only stopped services that are not disabled can be started",
                            )
                            .clicked()
                        {
                            request = Some(Pending::Control(ServiceControl::Start));
                        }
                        if ui
                            .add_enabled(s.can_stop, egui::Button::new("Stop"))
                            .on_disabled_hover_text(
                                "The service is not running or does not accept stop requests",
                            )
                            .clicked()
                        {
                            request = Some(Pending::Control(ServiceControl::Stop));
                        }
                        ui.separator();
                        ui.label("Startup type:");
                        let current = s.start_type;
                        let choosable = current.is_none_or(|c| StartType::CHOOSABLE.contains(&c));
                        ui.add_enabled_ui(choosable, |ui| {
                            egui::ComboBox::from_id_salt("service_start_type")
                                .selected_text(current.map_or(UNAVAILABLE, StartType::label))
                                .show_ui(ui, |ui| {
                                    for t in StartType::CHOOSABLE {
                                        if ui.selectable_label(current == Some(t), t.label()).clicked()
                                            && current != Some(t)
                                        {
                                            request = Some(Pending::StartType(t));
                                        }
                                    }
                                });
                        })
                        .response
                        .on_disabled_hover_text(
                            "Boot and System start types belong to drivers and are not changed here",
                        );
                        if let Some(pid) = s.pid
                            && ui.button("Show process").clicked()
                        {
                            actions.push(ServicesAction::ShowProcess(pid));
                        }
                    });
                    ui.add_space(4.0);
                    let deps = if s.dependencies.is_empty() {
                        "None".to_owned()
                    } else {
                        s.dependencies.join(", ")
                    };
                    let dependents: Vec<&str> = services::dependents(&self.services, &s.name)
                        .iter()
                        .map(|d| d.name.as_str())
                        .collect();
                    let dependents = if dependents.is_empty() {
                        "None".to_owned()
                    } else {
                        dependents.join(", ")
                    };
                    let or_unavailable =
                        |v: &Option<String>| v.clone().unwrap_or_else(|| UNAVAILABLE.to_owned());
                    key_values(
                        ui,
                        "service_kv",
                        &[
                            ("Status", s.state.label()),
                            (
                                "Process ID",
                                s.pid.map_or_else(|| UNAVAILABLE.to_owned(), |p| p.to_string()),
                            ),
                            (
                                "Hosting",
                                if s.shared_process {
                                    "Shared process (svchost group)".to_owned()
                                } else {
                                    "Own process".to_owned()
                                },
                            ),
                            ("Log on as", or_unavailable(&s.account)),
                            ("Executable", or_unavailable(&s.binary_path)),
                            ("Depends on", deps),
                            ("Required by", dependents),
                            ("Description", or_unavailable(&s.description)),
                        ],
                    );
                });
            });
        if let Some(p) = request {
            if confirm {
                self.pending = Some((s.name.clone(), p));
            } else {
                actions.push(to_action(s.name.clone(), p));
            }
        }
    }

    fn dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<ServicesAction>) {
        let Some((name, pending)) = &self.pending else {
            return;
        };
        let service = self.services.iter().find(|s| &s.name == name);
        let display = service.map_or(name.as_str(), |s| s.display_name.as_str());
        let (title, mut message, label, destructive) = match pending {
            Pending::Control(ServiceControl::Start) => (
                format!("Start {display}?"),
                format!("Windows will start the service \"{name}\" and any services it depends on."),
                "Start service".to_owned(),
                false,
            ),
            Pending::Control(ServiceControl::Stop) => (
                format!("Stop {display}?"),
                format!(
                    "Windows will stop the service \"{name}\". Features that rely on it will stop working until it \
                     is started again."
                ),
                "Stop service".to_owned(),
                true,
            ),
            Pending::StartType(t) => (
                format!("Change startup type of {display}?"),
                format!(
                    "The startup type of \"{name}\" will change from {} to {}. This changes Windows system \
                     configuration and persists across restarts.",
                    service
                        .and_then(|s| s.start_type)
                        .map_or(UNAVAILABLE, StartType::label),
                    t.label()
                ),
                "Change startup type".to_owned(),
                *t == StartType::Disabled,
            ),
        };
        if matches!(pending, Pending::Control(ServiceControl::Stop)) {
            let running: Vec<&str> = services::dependents(&self.services, name)
                .into_iter()
                .filter(|d| d.state.is_running())
                .map(|d| d.display_name.as_str())
                .collect();
            if !running.is_empty() {
                message.push_str(&format!(
                    "\n\nThese running services depend on it, and Windows will refuse the stop while they run:\n{}",
                    running.join("\n")
                ));
            }
            if service.is_some_and(|s| s.shared_process) {
                message.push_str(
                    "\n\nThe service shares its process with others; only this service is stopped.",
                );
            }
        }
        let dialog = Confirm {
            id: "confirm_service",
            title,
            message,
            confirm_label: label,
            destructive,
        };
        match dialog.show(ctx) {
            ConfirmResult::Pending => {}
            ConfirmResult::Cancelled => self.pending = None,
            ConfirmResult::Confirmed => {
                if let Some((name, p)) = self.pending.take() {
                    actions.push(to_action(name, p));
                }
            }
        }
    }
}

fn to_action(name: String, pending: Pending) -> ServicesAction {
    match pending {
        Pending::Control(control) => ServicesAction::Control { name, control },
        Pending::StartType(start) => ServicesAction::SetStartType { name, start },
    }
}
