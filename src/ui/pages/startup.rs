//! Startup programs from the registry Run keys and Startup folders, with
//! confirmed enable/disable. Entries are never deleted.

use std::cmp::Ordering;

use egui::RichText;

use crate::core::startup::{self, StartupEntry, StartupSource, StartupStatus};
use crate::ui::components::dialogs::{Confirm, ConfirmResult};
use crate::ui::components::list_table::{self, ListColumn, ListSort, ListTable};
use crate::ui::components::tables::key_values;
use crate::ui::components::{page_header, section};
use crate::ui::theme::Palette;
use crate::utils::formatting::UNAVAILABLE;

#[derive(Debug, Clone, PartialEq)]
pub enum StartupAction {
    Refresh,
    SetStatus {
        source: StartupSource,
        name: String,
        status: StartupStatus,
    },
    OpenLocation(String),
    Copy(String),
}

const COLUMNS: [ListColumn; 5] = [
    ListColumn {
        label: "Name",
        width: 200.0,
        tooltip: "",
    },
    ListColumn {
        label: "Publisher",
        width: 170.0,
        tooltip: "Company name from the program's version information",
    },
    ListColumn {
        label: "Status",
        width: 80.0,
        tooltip: "",
    },
    ListColumn {
        label: "Source",
        width: 220.0,
        tooltip: "Where Windows reads this entry from",
    },
    ListColumn {
        label: "Command",
        width: 260.0,
        tooltip: "",
    },
];

type EntryId = (StartupSource, String);

#[derive(Default)]
pub struct StartupPage {
    entries: Vec<StartupEntry>,
    error: Option<String>,
    loaded: bool,
    loading: bool,
    search: String,
    sort: ListSort,
    selected: Option<EntryId>,
    pending: Option<(EntryId, StartupStatus)>,
}

fn status_label(s: StartupStatus) -> &'static str {
    match s {
        StartupStatus::Enabled => "Enabled",
        StartupStatus::Disabled => "Disabled",
    }
}

fn compare(a: &StartupEntry, b: &StartupEntry, column: usize) -> Ordering {
    let lower = |s: Option<&str>| s.map(str::to_lowercase);
    match column {
        0 => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        1 => lower(a.publisher.as_deref()).cmp(&lower(b.publisher.as_deref())),
        2 => status_label(a.status).cmp(status_label(b.status)),
        3 => a.source.label().cmp(b.source.label()),
        _ => lower(a.command.as_deref().or(a.path.as_deref()))
            .cmp(&lower(b.command.as_deref().or(b.path.as_deref()))),
    }
}

impl StartupPage {
    pub fn set_loading(&mut self) {
        self.loading = true;
    }

    pub fn on_entries(&mut self, result: Result<Vec<StartupEntry>, String>) {
        self.loading = false;
        self.loaded = true;
        match result {
            Ok(list) => {
                self.entries = list;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, confirm: bool, elevated: Option<bool>) -> Vec<StartupAction> {
        let mut actions = Vec::new();
        if !self.loaded && !self.loading {
            actions.push(StartupAction::Refresh);
        }
        let palette = Palette::of(ui);
        let enabled = self
            .entries
            .iter()
            .filter(|e| e.status == StartupStatus::Enabled)
            .count();
        let subtitle =
            (!self.entries.is_empty()).then(|| format!("{} entries · {enabled} enabled", self.entries.len()));
        page_header(ui, "Startup", subtitle.as_deref());

        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Search name, publisher or command")
                    .desired_width(260.0),
            );
            if ui
                .add_enabled(!self.loading, egui::Button::new("Refresh"))
                .clicked()
            {
                actions.push(StartupAction::Refresh);
            }
            if self.loading {
                ui.spinner();
            }
        });
        ui.label(
            RichText::new(
                "Sources: registry Run and RunOnce keys and Startup folders. Scheduled tasks, services and \
                 shell extensions that start with Windows are not listed yet.",
            )
            .small()
            .color(palette.muted),
        );
        if let Some(e) = &self.error {
            ui.label(RichText::new(format!("Could not read startup entries: {e}")).color(palette.critical));
        }
        ui.add_space(4.0);
        if self.entries.is_empty() {
            if self.error.is_some() {
                return actions;
            }
            ui.label(
                RichText::new(if self.loaded {
                    "No startup entries found"
                } else {
                    "Loading startup entries…"
                })
                .color(palette.muted),
            );
            return actions;
        }

        let query = self.search.trim().to_lowercase();
        let mut view: Vec<usize> = (0..self.entries.len())
            .filter(|&i| startup::matches(&self.entries[i], &query))
            .collect();
        let sort = self.sort;
        view.sort_by(|&a, &b| {
            let ord = compare(&self.entries[a], &self.entries[b], sort.column);
            if sort.descending { ord.reverse() } else { ord }
        });
        let is_selected = |e: &StartupEntry, id: &Option<EntryId>| {
            id.as_ref().is_some_and(|(s, n)| *s == e.source && *n == e.name)
        };
        let selected_row = view
            .iter()
            .position(|&i| is_selected(&self.entries[i], &self.selected));
        let table_height = (ui.available_height() - 220.0).max(120.0);
        let events = list_table::show(
            ui,
            ListTable {
                id: "startup_table",
                columns: &COLUMNS,
                sort: self.sort,
                rows: view.len(),
                selected: selected_row,
                height: table_height,
            },
            |ui, row, column| {
                let e = &self.entries[view[row]];
                match column {
                    0 => list_table::text(ui, Some(&e.name)),
                    1 => list_table::text(ui, e.publisher.as_deref()),
                    2 => {
                        let color = match e.status {
                            StartupStatus::Enabled => palette.ok,
                            StartupStatus::Disabled => palette.muted,
                        };
                        ui.label(RichText::new(status_label(e.status)).color(color));
                    }
                    3 => list_table::text(ui, Some(e.source.label())),
                    _ => list_table::text(ui, e.command.as_deref().or(e.path.as_deref())),
                }
            },
        );
        if let Some(c) = events.sort {
            self.sort.toggle(c);
        }
        if let Some(row) = events.clicked.or(events.double_clicked) {
            let e = &self.entries[view[row]];
            self.selected = Some((e.source, e.name.clone()));
        }

        ui.add_space(6.0);
        self.properties(ui, confirm, elevated, &mut actions);
        self.dialog(ui.ctx(), &mut actions);
        actions
    }

    fn properties(
        &mut self,
        ui: &mut egui::Ui,
        confirm: bool,
        elevated: Option<bool>,
        actions: &mut Vec<StartupAction>,
    ) {
        let palette = Palette::of(ui);
        let entry = self
            .selected
            .as_ref()
            .and_then(|(s, n)| self.entries.iter().find(|e| e.source == *s && e.name == *n))
            .cloned();
        let Some(e) = entry else {
            section(ui, "DETAILS", |ui| {
                ui.label(RichText::new("Select an entry to see its details").color(palette.muted));
            });
            return;
        };
        let mut request = None;
        egui::ScrollArea::vertical()
            .id_salt("startup_props")
            .show(ui, |ui| {
                section(ui, "DETAILS", |ui| {
                    ui.strong(&e.name);
                    ui.horizontal(|ui| {
                        let target = match e.status {
                            StartupStatus::Enabled => StartupStatus::Disabled,
                            StartupStatus::Disabled => StartupStatus::Enabled,
                        };
                        let label = match target {
                            StartupStatus::Enabled => "Enable",
                            StartupStatus::Disabled => "Disable",
                        };
                        if ui
                            .add_enabled(e.source.supports_toggle(), egui::Button::new(label))
                            .on_disabled_hover_text(
                                "RunOnce entries run a single time and cannot be disabled",
                            )
                            .clicked()
                        {
                            request = Some(target);
                        }
                        if let Some(path) = &e.path
                            && ui.button("Open file location").clicked()
                        {
                            actions.push(StartupAction::OpenLocation(path.clone()));
                        }
                        if let Some(cmd) = e.command.as_ref().or(e.path.as_ref())
                            && ui.button("Copy command").clicked()
                        {
                            actions.push(StartupAction::Copy(cmd.clone()));
                        }
                    });
                    if e.source.is_machine_wide() && elevated != Some(true) {
                        ui.label(
                            RichText::new(
                                "This entry applies to all users. Changing it requires administrator rights.",
                            )
                            .small()
                            .color(palette.warning),
                        );
                    }
                    ui.add_space(4.0);
                    let or_unavailable =
                        |v: &Option<String>| v.clone().unwrap_or_else(|| UNAVAILABLE.to_owned());
                    key_values(
                        ui,
                        "startup_kv",
                        &[
                            ("Status", status_label(e.status).to_owned()),
                            ("Source", e.source.label().to_owned()),
                            ("Location", e.location.clone()),
                            ("Command", or_unavailable(&e.command)),
                            ("Program", or_unavailable(&e.path)),
                            ("Publisher", or_unavailable(&e.publisher)),
                            ("Description", or_unavailable(&e.description)),
                        ],
                    );
                });
            });
        if let Some(status) = request {
            let id = (e.source, e.name.clone());
            if confirm {
                self.pending = Some((id, status));
            } else {
                actions.push(StartupAction::SetStatus {
                    source: id.0,
                    name: id.1,
                    status,
                });
            }
        }
    }

    fn dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<StartupAction>) {
        let Some(((source, name), status)) = &self.pending else {
            return;
        };
        let (title, message, label) = match status {
            StartupStatus::Disabled => (
                format!("Disable {name} at startup?"),
                format!(
                    "\"{name}\" will no longer start when you sign in. The entry is kept and can be enabled again \
                     here or in Windows Settings.\n\nSource: {}",
                    source.label()
                ),
                "Disable",
            ),
            StartupStatus::Enabled => (
                format!("Enable {name} at startup?"),
                format!(
                    "\"{name}\" will start automatically when you sign in.\n\nSource: {}",
                    source.label()
                ),
                "Enable",
            ),
        };
        let dialog = Confirm {
            id: "confirm_startup",
            title,
            message,
            confirm_label: label.into(),
            destructive: false,
        };
        match dialog.show(ctx) {
            ConfirmResult::Pending => {}
            ConfirmResult::Cancelled => self.pending = None,
            ConfirmResult::Confirmed => {
                if let Some(((source, name), status)) = self.pending.take() {
                    actions.push(StartupAction::SetStatus { source, name, status });
                }
            }
        }
    }
}
