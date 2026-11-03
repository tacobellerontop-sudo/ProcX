//! Detailed inspection of one process instance.

use egui::RichText;
use egui_extras::{Column, TableBuilder};

use crate::core::models::{ProcessInfo, ProcessKey, percent};
use crate::core::series::TimeSeries;
use crate::process::details::ProcessDetails;
use crate::process::tree;
use crate::ui::components::charts::{ChartOptions, ChartSeries, live_chart};
use crate::ui::components::tables::key_values;
use crate::ui::components::{page_header, section};
use crate::ui::theme::Palette;
use crate::utils::formatting::{self as fmt, UNAVAILABLE};
use crate::utils::time;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Performance,
    Threads,
    Modules,
    Family,
}

impl Tab {
    const ALL: [Tab; 5] = [
        Tab::General,
        Tab::Performance,
        Tab::Threads,
        Tab::Modules,
        Tab::Family,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Performance => "Performance",
            Tab::Threads => "Threads",
            Tab::Modules => "Modules",
            Tab::Family => "Parent and children",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DetailsAction {
    Inspect(ProcessKey),
    OpenLocation(String),
    /// Inspect another process (e.g. parent or child).
    Select(ProcessKey),
}

/// Samples kept for the per-process charts.
const SERIES_CAPACITY: usize = 600;

pub struct DetailsPage {
    key: Option<ProcessKey>,
    last_seen: Option<ProcessInfo>,
    exited: bool,
    details: Option<ProcessDetails>,
    loading: bool,
    tab: Tab,
    cpu: TimeSeries,
    memory: TimeSeries,
    module_filter: String,
}

impl Default for DetailsPage {
    fn default() -> Self {
        Self {
            key: None,
            last_seen: None,
            exited: false,
            details: None,
            loading: false,
            tab: Tab::General,
            cpu: TimeSeries::new(SERIES_CAPACITY),
            memory: TimeSeries::new(SERIES_CAPACITY),
            module_filter: String::new(),
        }
    }
}

fn opt<T: ToString>(v: Option<T>) -> String {
    v.map_or_else(|| UNAVAILABLE.to_owned(), |v| v.to_string())
}

impl DetailsPage {
    pub fn target(&self) -> Option<ProcessKey> {
        self.key
    }

    /// Switches to a process. Returns true if inspection should be requested.
    pub fn set_target(&mut self, key: ProcessKey, processes: Option<&[ProcessInfo]>) -> bool {
        if self.key == Some(key) {
            return false;
        }
        *self = Self {
            key: Some(key),
            tab: self.tab,
            loading: true,
            ..Self::default()
        };
        self.last_seen = processes.and_then(|ps| ps.iter().find(|p| p.key == key).cloned());
        true
    }

    pub fn on_snapshot(&mut self, processes: &[ProcessInfo], t: f64) {
        let Some(key) = self.key else { return };
        match processes.iter().find(|p| p.key == key) {
            Some(p) => {
                if let Some(cpu) = p.cpu_percent {
                    self.cpu.push(t, f64::from(cpu));
                }
                if let Some(ws) = p.working_set {
                    self.memory.push(t, ws as f64);
                }
                self.last_seen = Some(p.clone());
                self.exited = false;
            }
            None => self.exited = true,
        }
    }

    pub fn on_details(&mut self, key: ProcessKey, details: ProcessDetails) {
        if self.key == Some(key) {
            self.details = Some(details);
            self.loading = false;
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        processes: Option<&[ProcessInfo]>,
        total_memory: u64,
        chart_now: f64,
        chart_window: f64,
    ) -> Vec<DetailsAction> {
        let mut actions = Vec::new();
        let palette = Palette::of(ui);
        let Some(p) = self.last_seen.clone() else {
            page_header(ui, "Details", None);
            ui.label(
                RichText::new(
                    "Select a process on the Processes page, then choose Properties, double-click it or press Enter.",
                )
                .color(palette.muted),
            );
            return actions;
        };

        page_header(ui, &format!("{} (PID {})", p.name, p.key.pid), None);
        if self.exited {
            ui.label(
                RichText::new("This process has exited. Showing the last known information.")
                    .color(palette.warning),
            );
        }
        ui.horizontal(|ui| {
            for tab in Tab::ALL {
                ui.selectable_value(&mut self.tab, tab, tab.label());
            }
            ui.separator();
            if ui
                .add_enabled(
                    !self.exited && !self.loading,
                    egui::Button::new("Refresh details"),
                )
                .on_hover_text("Re-read command line, threads and modules")
                .clicked()
            {
                self.loading = true;
                actions.push(DetailsAction::Inspect(p.key));
            }
            if self.loading {
                ui.spinner();
            }
        });
        ui.add_space(6.0);

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| match self.tab {
                Tab::General => self.general(ui, &p, &mut actions),
                Tab::Performance => self.performance(ui, &p, total_memory, chart_now, chart_window),
                Tab::Threads => self.threads(ui),
                Tab::Modules => self.modules(ui),
                Tab::Family => family(ui, &p, processes.unwrap_or_default(), &mut actions),
            });
        actions
    }

    fn general(&self, ui: &mut egui::Ui, p: &ProcessInfo, actions: &mut Vec<DetailsAction>) {
        let command_line = match self.details.as_ref().map(|d| &d.command_line) {
            Some(Ok(c)) => c.clone(),
            Some(Err(e)) => format!("{UNAVAILABLE} ({e})"),
            None => "Loading…".into(),
        };
        let started = p.started_at.map_or_else(
            || UNAVAILABLE.to_owned(),
            |t| {
                format!(
                    "{} ({})",
                    time::format_utc(t),
                    time::relative(t, std::time::SystemTime::now())
                )
            },
        );
        let file_field = |f: fn(&crate::platform::FileVersionInfo) -> &Option<String>| match &self.details {
            None => "Loading…".to_owned(),
            Some(d) => d
                .file_info
                .as_ref()
                .and_then(|i| f(i).clone())
                .unwrap_or_else(|| UNAVAILABLE.to_owned()),
        };
        section(ui, "GENERAL", |ui| {
            key_values(
                ui,
                "details_general",
                &[
                    ("Name", p.name.to_string()),
                    ("Description", file_field(|i| &i.description)),
                    ("Company", file_field(|i| &i.company)),
                    ("File version", file_field(|i| &i.version)),
                    ("PID", p.key.pid.to_string()),
                    ("Parent PID", opt(p.parent_pid)),
                    (
                        "Executable",
                        p.exe_path.as_deref().unwrap_or(UNAVAILABLE).to_owned(),
                    ),
                    ("Command line", command_line),
                    ("Started", started),
                    ("User", p.user.as_deref().unwrap_or(UNAVAILABLE).to_owned()),
                    ("Architecture", opt(p.architecture.map(|a| a.label()))),
                    ("Session", opt(p.session_id)),
                    ("Status", p.status.label().to_owned()),
                    (
                        "Critical",
                        match p.critical {
                            Some(true) => "Yes (ending it would crash Windows)".into(),
                            Some(false) => "No".into(),
                            None => UNAVAILABLE.into(),
                        },
                    ),
                ],
            );
            if let Some(path) = &p.exe_path {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Open file location").clicked() {
                        actions.push(DetailsAction::OpenLocation(path.to_string()));
                    }
                    if ui.button("Copy path").clicked() {
                        ui.ctx().copy_text(path.to_string());
                    }
                });
            }
        });
    }

    fn performance(&self, ui: &mut egui::Ui, p: &ProcessInfo, total_memory: u64, now: f64, window: f64) {
        let palette = Palette::of(ui);
        section(ui, "PERFORMANCE", |ui| {
            key_values(
                ui,
                "details_perf",
                &[
                    ("CPU", opt(p.cpu_percent.map(fmt::percent))),
                    (
                        "CPU time",
                        opt(p.cpu_time.map(|t| format!("{:.2} s", t.as_secs_f64()))),
                    ),
                    (
                        "Working set",
                        opt(p
                            .working_set
                            .map(|w| format!("{} ({:.1}% of RAM)", fmt::bytes(w), percent(w, total_memory)))),
                    ),
                    ("Peak working set", opt(p.peak_working_set.map(fmt::bytes))),
                    ("Private bytes", opt(p.private_bytes.map(fmt::bytes))),
                    ("Threads", opt(p.threads)),
                    ("Handles", opt(p.handles)),
                ],
            );
        });
        ui.add_space(6.0);
        ui.columns(2, |cols| {
            section(&mut cols[0], "CPU (since selected)", |ui| {
                live_chart(
                    ui,
                    "proc_cpu",
                    now,
                    &[ChartSeries {
                        name: "CPU",
                        data: &self.cpu,
                        color: palette.cpu,
                    }],
                    &ChartOptions {
                        height: 120.0,
                        window_secs: window,
                        y_max: None,
                        format_y: |v| format!("{v:.1}%"),
                    },
                );
            });
            section(&mut cols[1], "WORKING SET (since selected)", |ui| {
                live_chart(
                    ui,
                    "proc_mem",
                    now,
                    &[ChartSeries {
                        name: "Working set",
                        data: &self.memory,
                        color: palette.memory,
                    }],
                    &ChartOptions {
                        height: 120.0,
                        window_secs: window,
                        y_max: None,
                        format_y: |v| fmt::bytes(v.max(0.0) as u64),
                    },
                );
            });
        });
    }

    fn threads(&self, ui: &mut egui::Ui) {
        let Some(details) = &self.details else {
            ui.label("Loading…");
            return;
        };
        match &details.threads {
            Err(e) => {
                ui.label(RichText::new(e).color(Palette::of(ui).warning));
            }
            Ok(threads) => {
                ui.label(
                    RichText::new(format!(
                        "{} threads. Thread states are not reported by the API used; priorities are shown where the thread could be opened.",
                        threads.len()
                    ))
                    .small()
                    .color(Palette::of(ui).muted),
                );
                TableBuilder::new(ui)
                    .id_salt("threads")
                    .striped(true)
                    .resizable(true)
                    .column(Column::initial(100.0))
                    .column(Column::initial(110.0))
                    .column(Column::remainder())
                    .header(20.0, |mut h| {
                        h.col(|ui| {
                            ui.strong("Thread ID");
                        });
                        h.col(|ui| {
                            ui.strong("Base priority");
                        });
                        h.col(|ui| {
                            ui.strong("Relative priority");
                        });
                    })
                    .body(|body| {
                        body.rows(18.0, threads.len(), |mut row| {
                            let t = &threads[row.index()];
                            row.col(|ui| {
                                ui.monospace(t.tid.to_string());
                            });
                            row.col(|ui| {
                                ui.monospace(t.base_priority.to_string());
                            });
                            row.col(|ui| {
                                ui.monospace(opt(t.priority));
                            });
                        });
                    });
            }
        }
    }

    fn modules(&mut self, ui: &mut egui::Ui) {
        let Some(details) = &self.details else {
            ui.label("Loading…");
            return;
        };
        match &details.modules {
            Err(e) => {
                ui.label(RichText::new(e).color(Palette::of(ui).warning));
            }
            Ok(modules) => {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.module_filter)
                            .hint_text("Filter modules")
                            .desired_width(240.0),
                    );
                    ui.label(
                        RichText::new(format!("{} modules", modules.len()))
                            .small()
                            .color(Palette::of(ui).muted),
                    );
                });
                let needle = self.module_filter.to_lowercase();
                let shown: Vec<_> = modules
                    .iter()
                    .filter(|m| {
                        needle.is_empty()
                            || m.name.to_lowercase().contains(&needle)
                            || m.path.to_lowercase().contains(&needle)
                    })
                    .collect();
                TableBuilder::new(ui)
                    .id_salt("modules")
                    .striped(true)
                    .resizable(true)
                    .column(Column::initial(180.0))
                    .column(Column::initial(140.0))
                    .column(Column::initial(80.0))
                    .column(Column::remainder())
                    .header(20.0, |mut h| {
                        h.col(|ui| {
                            ui.strong("Module");
                        });
                        h.col(|ui| {
                            ui.strong("Base address");
                        });
                        h.col(|ui| {
                            ui.strong("Size");
                        });
                        h.col(|ui| {
                            ui.strong("Path");
                        });
                    })
                    .body(|body| {
                        body.rows(18.0, shown.len(), |mut row| {
                            let m = shown[row.index()];
                            row.col(|ui| {
                                ui.label(&m.name);
                            });
                            row.col(|ui| {
                                ui.monospace(format!("0x{:016x}", m.base_address));
                            });
                            row.col(|ui| {
                                ui.monospace(fmt::bytes(u64::from(m.size)));
                            });
                            row.col(|ui| {
                                ui.add(egui::Label::new(&m.path).truncate());
                            });
                        });
                    });
            }
        }
    }
}

fn family(ui: &mut egui::Ui, p: &ProcessInfo, processes: &[ProcessInfo], actions: &mut Vec<DetailsAction>) {
    let palette = Palette::of(ui);
    let parents = tree::parent_indices(processes);
    let index = processes.iter().position(|x| x.key == p.key);
    let mut link = |ui: &mut egui::Ui, q: &ProcessInfo, depth: usize| {
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 14.0);
            if ui.link(format!("{} (PID {})", q.name, q.key.pid)).clicked() {
                actions.push(DetailsAction::Select(q.key));
            }
            if let Some(path) = &q.exe_path {
                ui.label(RichText::new(path.as_ref()).small().color(palette.muted));
            }
        });
    };

    section(ui, "PARENT", |ui| {
        match (p.parent_pid, index.and_then(|i| parents[i])) {
            (_, Some(parent)) => link(ui, &processes[parent], 0),
            (Some(ppid), None) => {
                ui.label(
                    RichText::new(format!(
                        "PID {ppid} has exited (or the PID was reused by an unrelated process)"
                    ))
                    .color(palette.muted),
                );
            }
            (None, None) => {
                ui.label(RichText::new("No parent").color(palette.muted));
            }
        }
    });
    ui.add_space(6.0);
    section(ui, "CHILDREN (TREE)", |ui| {
        let Some(root) = index else {
            ui.label(RichText::new("The process has exited").color(palette.muted));
            return;
        };
        let mut any = false;
        let mut stack: Vec<(usize, usize)> = parents
            .iter()
            .enumerate()
            .filter(|(_, par)| **par == Some(root))
            .map(|(i, _)| (i, 0))
            .collect();
        let mut guard = 0;
        while let Some((i, depth)) = stack.pop() {
            guard += 1;
            if guard > processes.len() {
                break;
            }
            any = true;
            link(ui, &processes[i], depth);
            stack.extend(
                parents
                    .iter()
                    .enumerate()
                    .filter(|(_, par)| **par == Some(i))
                    .map(|(c, _)| (c, depth + 1)),
            );
        }
        if !any {
            ui.label(RichText::new("No child processes").color(palette.muted));
        }
    });
}
