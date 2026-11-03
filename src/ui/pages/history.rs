//! Recorded history: system charts and per-process history over a chosen
//! range, plus CSV export.

use std::cmp::Ordering;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use egui::RichText;

use crate::history::accumulate::unix_ms;
use crate::history::store::{ProcessPoint, ProcessRow, SystemPoint};
use crate::history::worker::{HistoryStatus, MAX_POINTS};
use crate::ui::components::charts::{HistoryChartOptions, HistorySeries, history_chart, split_segments};
use crate::ui::components::list_table::{self, ListColumn, ListSort, ListTable};
use crate::ui::components::{page_header, section};
use crate::ui::theme::Palette;
use crate::utils::formatting::{self as fmt, UNAVAILABLE};
use crate::utils::time::format_clock;

#[derive(Debug, Clone, PartialEq)]
pub enum HistoryAction {
    Query {
        from_ms: i64,
        to_ms: i64,
    },
    ProcessSeries {
        id: i64,
        from_ms: i64,
        to_ms: i64,
    },
    ExportSystem {
        from_ms: i64,
        to_ms: i64,
    },
    ExportProcess {
        row: Box<ProcessRow>,
        from_ms: i64,
        to_ms: i64,
    },
    ShowFile(String),
    OpenSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Range {
    Hour,
    SixHours,
    Day,
    Week,
    Month,
}

impl Range {
    const ALL: [Range; 5] = [
        Range::Hour,
        Range::SixHours,
        Range::Day,
        Range::Week,
        Range::Month,
    ];

    fn label(self) -> &'static str {
        match self {
            Range::Hour => "1 hour",
            Range::SixHours => "6 hours",
            Range::Day => "24 hours",
            Range::Week => "7 days",
            Range::Month => "30 days",
        }
    }

    fn duration(self) -> Duration {
        let hours = match self {
            Range::Hour => 1,
            Range::SixHours => 6,
            Range::Day => 24,
            Range::Week => 24 * 7,
            Range::Month => 24 * 30,
        };
        Duration::from_secs(hours * 3600)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    System,
    Processes,
}

const COLUMNS: [ListColumn; 8] = [
    ListColumn {
        label: "Name",
        width: 170.0,
        tooltip: "",
    },
    ListColumn {
        label: "PID",
        width: 60.0,
        tooltip: "",
    },
    ListColumn {
        label: "First recorded",
        width: 120.0,
        tooltip: "First time ProcX saw this process instance (UTC)",
    },
    ListColumn {
        label: "Last recorded",
        width: 120.0,
        tooltip: "Last time ProcX saw this process instance (UTC)",
    },
    ListColumn {
        label: "Avg CPU",
        width: 70.0,
        tooltip: "Average over the recorded samples in this range",
    },
    ListColumn {
        label: "Peak CPU",
        width: 70.0,
        tooltip: "Highest interval average in this range",
    },
    ListColumn {
        label: "Peak memory",
        width: 90.0,
        tooltip: "Highest working set in this range",
    },
    ListColumn {
        label: "Samples",
        width: 60.0,
        tooltip: "Intervals in which this process was among the busiest and was sampled",
    },
];

pub struct HistoryPage {
    range: Range,
    tab: Tab,
    window: Option<(i64, i64)>,
    system: Option<Result<Vec<SystemPoint>, String>>,
    processes: Option<Result<Vec<ProcessRow>, String>>,
    loading: bool,
    queried_at: Option<Instant>,
    status: Option<HistoryStatus>,
    search: String,
    sort: ListSort,
    selected: Option<i64>,
    series: Option<(i64, Result<Vec<ProcessPoint>, String>)>,
    exported: Option<Result<PathBuf, String>>,
}

impl Default for HistoryPage {
    fn default() -> Self {
        Self {
            range: Range::Hour,
            tab: Tab::System,
            window: None,
            system: None,
            processes: None,
            loading: false,
            queried_at: None,
            status: None,
            search: String::new(),
            sort: ListSort {
                column: 5,
                descending: true,
            },
            selected: None,
            series: None,
            exported: None,
        }
    }
}

fn opt_cmp<T: PartialOrd>(a: Option<T>, b: Option<T>) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

fn compare(a: &ProcessRow, b: &ProcessRow, column: usize) -> Ordering {
    match column {
        0 => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        1 => a.pid.cmp(&b.pid),
        2 => a.first_seen_ms.cmp(&b.first_seen_ms),
        3 => a.last_seen_ms.cmp(&b.last_seen_ms),
        4 => opt_cmp(a.cpu_avg, b.cpu_avg),
        5 => opt_cmp(a.cpu_max, b.cpu_max),
        6 => opt_cmp(a.working_set_max, b.working_set_max),
        _ => a.samples.cmp(&b.samples),
    }
}

fn clock(ts_ms: i64, with_date: bool) -> String {
    format_clock(ts_ms.div_euclid(1000), with_date)
}

impl HistoryPage {
    pub fn on_status(&mut self, status: HistoryStatus) {
        self.status = Some(status);
    }

    pub fn on_range(
        &mut self,
        from_ms: i64,
        to_ms: i64,
        system: Result<Vec<SystemPoint>, String>,
        processes: Result<Vec<ProcessRow>, String>,
    ) {
        self.loading = false;
        self.window = Some((from_ms, to_ms));
        self.system = Some(system);
        self.processes = Some(processes);
    }

    pub fn on_series(&mut self, id: i64, points: Result<Vec<ProcessPoint>, String>) {
        if self.selected == Some(id) {
            self.series = Some((id, points));
        }
    }

    pub fn on_exported(&mut self, result: Result<PathBuf, String>) {
        self.exported = Some(result);
    }

    fn query(&mut self, actions: &mut Vec<HistoryAction>) {
        let to_ms = unix_ms(SystemTime::now());
        let from_ms = to_ms - self.range.duration().as_millis() as i64;
        self.loading = true;
        self.queried_at = Some(Instant::now());
        actions.push(HistoryAction::Query { from_ms, to_ms });
        if let Some(id) = self.selected {
            actions.push(HistoryAction::ProcessSeries { id, from_ms, to_ms });
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, record_interval_secs: u32) -> Vec<HistoryAction> {
        let mut actions = Vec::new();
        // Re-query on open and then once per recording interval while visible.
        let refresh = Duration::from_secs(u64::from(record_interval_secs.max(5)));
        if !self.loading && self.queried_at.is_none_or(|t| t.elapsed() >= refresh) {
            self.query(&mut actions);
        }
        ui.ctx().request_repaint_after(refresh);

        let palette = Palette::of(ui);
        let subtitle = match &self.status {
            Some(s) if s.enabled => format!("Recording every {record_interval_secs} s"),
            Some(_) => "Recording is off".to_owned(),
            None => String::new(),
        };
        page_header(ui, "History", Some(&subtitle));

        ui.horizontal(|ui| {
            for r in Range::ALL {
                if ui.selectable_label(self.range == r, r.label()).clicked() && self.range != r {
                    self.range = r;
                    self.query(&mut actions);
                }
            }
            ui.separator();
            if ui
                .add_enabled(!self.loading, egui::Button::new("Refresh"))
                .clicked()
            {
                self.query(&mut actions);
            }
            if self.loading {
                ui.spinner();
            }
            ui.separator();
            ui.selectable_value(&mut self.tab, Tab::System, "System");
            ui.selectable_value(&mut self.tab, Tab::Processes, "Processes");
        });
        self.status_line(ui, &mut actions);
        ui.add_space(4.0);

        let Some((from_ms, to_ms)) = self.window else {
            ui.label(RichText::new("Loading history…").color(palette.muted));
            return actions;
        };
        // Lines break where consecutive samples are further apart than this.
        let bucket_ms = (to_ms - from_ms) / MAX_POINTS as i64;
        let gap_ms = (i64::from(record_interval_secs) * 1000 * 3).max(bucket_ms * 3);
        match self.tab {
            Tab::System => self.system_tab(ui, from_ms, to_ms, gap_ms, &mut actions),
            Tab::Processes => self.processes_tab(ui, from_ms, to_ms, gap_ms, &mut actions),
        }
        actions
    }

    fn status_line(&mut self, ui: &mut egui::Ui, actions: &mut Vec<HistoryAction>) {
        let palette = Palette::of(ui);
        let Some(status) = &self.status else {
            return;
        };
        ui.horizontal_wrapped(|ui| {
            let mut parts = vec![format!("Database: {}", status.path.display())];
            if let Some(stats) = &status.stats {
                if let Some(bytes) = stats.file_bytes {
                    parts.push(fmt::bytes(bytes));
                }
                parts.push(format!("{} system samples", fmt::thousands(stats.system_rows)));
                parts.push(format!(
                    "{} process instances",
                    fmt::thousands(stats.process_rows)
                ));
                if let Some(oldest) = stats.oldest_ms {
                    parts.push(format!("oldest {} UTC", clock(oldest, true)));
                }
            }
            ui.label(RichText::new(parts.join(" · ")).small().color(palette.muted));
            if ui.small_button("Settings").clicked() {
                actions.push(HistoryAction::OpenSettings);
            }
        });
        if let Some(e) = &status.error {
            ui.label(RichText::new(format!("History database unavailable: {e}")).color(palette.critical));
        }
        if !status.enabled {
            ui.label(
                RichText::new(
                    "Recording is off. Existing history can still be viewed; enable recording in Settings.",
                )
                .small()
                .color(palette.warning),
            );
        }
        match &self.exported {
            Some(Ok(path)) => {
                let path = path.display().to_string();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Exported {path}"))
                            .small()
                            .color(palette.ok),
                    );
                    if ui.small_button("Show in folder").clicked() {
                        actions.push(HistoryAction::ShowFile(path.clone()));
                    }
                });
            }
            Some(Err(e)) => {
                ui.label(
                    RichText::new(format!("Export failed: {e}"))
                        .small()
                        .color(palette.critical),
                );
            }
            None => {}
        }
    }

    fn system_tab(
        &mut self,
        ui: &mut egui::Ui,
        from_ms: i64,
        to_ms: i64,
        gap_ms: i64,
        actions: &mut Vec<HistoryAction>,
    ) {
        let palette = Palette::of(ui);
        let points = match &self.system {
            Some(Ok(p)) => p,
            Some(Err(e)) => {
                ui.label(RichText::new(format!("Could not read history: {e}")).color(palette.critical));
                return;
            }
            None => return,
        };
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} points from {} to {} UTC. Drag or scroll to pan and zoom; double-click to reset.",
                    points.len(),
                    clock(from_ms, true),
                    clock(to_ms, true)
                ))
                .small()
                .color(palette.muted),
            );
            if ui
                .add_enabled(!points.is_empty(), egui::Button::new("Export CSV"))
                .on_hover_text("Writes every recorded sample in this range (not the thinned chart points)")
                .clicked()
            {
                actions.push(HistoryAction::ExportSystem { from_ms, to_ms });
            }
        });
        if points.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "No samples in this range yet. ProcX writes one sample per recording interval while it is \
                     running; periods when it was closed appear as gaps.",
                )
                .color(palette.muted),
            );
            return;
        }
        let series = |name: &str, color, thin, f: &dyn Fn(&SystemPoint) -> Option<f64>| HistorySeries {
            name: name.to_owned(),
            color,
            thin,
            segments: split_segments(points.iter().map(|p| (p.ts_ms, f(p))), gap_ms),
        };
        let opts = |y_max, format_y| HistoryChartOptions {
            height: 150.0,
            from_secs: from_ms as f64 / 1000.0,
            to_secs: to_ms as f64 / 1000.0,
            y_max,
            format_y,
        };
        egui::ScrollArea::vertical()
            .id_salt("history_system")
            .show(ui, |ui| {
                section(ui, "CPU", |ui| {
                    history_chart(
                        ui,
                        "hist_cpu",
                        "history",
                        &[
                            series("Average", palette.cpu, false, &|p| Some(p.cpu_avg)),
                            series("Peak", palette.warning, true, &|p| Some(p.cpu_max)),
                        ],
                        &opts(Some(100.0), |v| format!("{v:.0}%")),
                    );
                });
                ui.add_space(6.0);
                section(ui, "MEMORY", |ui| {
                    history_chart(
                        ui,
                        "hist_mem",
                        "history",
                        &[
                            series("In use", palette.memory, false, &|p| Some(p.mem_used)),
                            series("Committed", palette.accent, true, &|p| p.commit),
                        ],
                        &opts(points.iter().map(|p| p.mem_total).reduce(f64::max), |v| {
                            fmt::bytes(v.max(0.0) as u64)
                        }),
                    );
                });
                ui.add_space(6.0);
                section(ui, "DISK", |ui| {
                    history_chart(
                        ui,
                        "hist_disk",
                        "history",
                        &[
                            series("Read", palette.disk_read, false, &|p| Some(p.disk_read)),
                            series("Write", palette.disk_write, false, &|p| Some(p.disk_write)),
                        ],
                        &opts(None, |v| fmt::rate(v.max(0.0))),
                    );
                });
                ui.add_space(6.0);
                section(ui, "NETWORK", |ui| {
                    history_chart(
                        ui,
                        "hist_net",
                        "history",
                        &[
                            series("Received", palette.net_rx, false, &|p| Some(p.net_rx)),
                            series("Sent", palette.net_tx, false, &|p| Some(p.net_tx)),
                        ],
                        &opts(None, |v| fmt::rate(v.max(0.0))),
                    );
                });
            });
    }

    fn processes_tab(
        &mut self,
        ui: &mut egui::Ui,
        from_ms: i64,
        to_ms: i64,
        gap_ms: i64,
        actions: &mut Vec<HistoryAction>,
    ) {
        let palette = Palette::of(ui);
        let rows = match &self.processes {
            Some(Ok(r)) => r,
            Some(Err(e)) => {
                ui.label(RichText::new(format!("Could not read history: {e}")).color(palette.critical));
                return;
            }
            None => return,
        };
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Search name, PID or path")
                    .desired_width(240.0),
            );
            ui.label(
                RichText::new(
                    "Lifetimes are recorded for every process; CPU and memory samples only for the busiest \
                     processes in each interval.",
                )
                .small()
                .color(palette.muted),
            );
        });
        if rows.is_empty() {
            ui.label(RichText::new("No processes recorded in this range yet").color(palette.muted));
            return;
        }
        let query = self.search.trim().to_lowercase();
        let mut view: Vec<usize> = (0..rows.len())
            .filter(|&i| {
                let r = &rows[i];
                query.is_empty()
                    || r.name.to_lowercase().contains(&query)
                    || r.pid.to_string() == query
                    || r.exe_path
                        .as_deref()
                        .is_some_and(|p| p.to_lowercase().contains(&query))
            })
            .collect();
        let sort = self.sort;
        view.sort_by(|&a, &b| {
            let o = compare(&rows[a], &rows[b], sort.column);
            if sort.descending { o.reverse() } else { o }
        });
        let selected_row = view.iter().position(|&i| Some(rows[i].id) == self.selected);
        let with_date = to_ms - from_ms > 86_400_000;
        let height = (ui.available_height() - if self.selected.is_some() { 330.0 } else { 40.0 }).max(120.0);
        let events = list_table::show(
            ui,
            ListTable {
                id: "history_processes",
                columns: &COLUMNS,
                sort: self.sort,
                rows: view.len(),
                selected: selected_row,
                height,
            },
            |ui, row, column| {
                let r = &rows[view[row]];
                let mono = |ui: &mut egui::Ui, v: Option<String>| match v {
                    Some(v) => {
                        ui.label(RichText::new(v).monospace());
                    }
                    None => {
                        ui.label(RichText::new(UNAVAILABLE).color(palette.muted))
                            .on_hover_text("Not sampled in this range");
                    }
                };
                match column {
                    0 => {
                        ui.add(egui::Label::new(&r.name).truncate())
                            .on_hover_text(r.exe_path.as_deref().unwrap_or(&r.name));
                    }
                    1 => mono(ui, Some(r.pid.to_string())),
                    2 => mono(ui, Some(clock(r.first_seen_ms, with_date))),
                    3 => mono(ui, Some(clock(r.last_seen_ms, with_date))),
                    4 => mono(ui, r.cpu_avg.map(|v| fmt::percent(v as f32))),
                    5 => mono(ui, r.cpu_max.map(|v| fmt::percent(v as f32))),
                    6 => mono(ui, r.working_set_max.map(fmt::bytes)),
                    _ => mono(ui, Some(r.samples.to_string())),
                }
            },
        );
        if let Some(c) = events.sort {
            self.sort.toggle(c);
        }
        if let Some(row) = events.clicked.or(events.double_clicked) {
            let id = rows[view[row]].id;
            if self.selected != Some(id) {
                self.selected = Some(id);
                self.series = None;
                actions.push(HistoryAction::ProcessSeries { id, from_ms, to_ms });
            }
        }

        let Some(row) = self
            .selected
            .and_then(|id| rows.iter().find(|r| r.id == id))
            .cloned()
        else {
            return;
        };
        ui.add_space(6.0);
        section(ui, "SELECTED PROCESS", |ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("{} (PID {})", row.name, row.pid));
                if let Some(user) = &row.user {
                    ui.label(RichText::new(user).color(palette.muted));
                }
                if ui
                    .add_enabled(row.samples > 0, egui::Button::new("Export CSV"))
                    .clicked()
                {
                    actions.push(HistoryAction::ExportProcess {
                        row: Box::new(row.clone()),
                        from_ms,
                        to_ms,
                    });
                }
            });
            if let Some(path) = &row.exe_path {
                ui.label(RichText::new(path).small().color(palette.muted));
            }
            let points = match &self.series {
                Some((id, Ok(p))) if *id == row.id => p,
                Some((id, Err(e))) if *id == row.id => {
                    ui.label(RichText::new(format!("Could not read samples: {e}")).color(palette.critical));
                    return;
                }
                _ => {
                    ui.spinner();
                    return;
                }
            };
            if points.is_empty() {
                ui.label(
                    RichText::new(
                        "This process was recorded but never among the sampled top consumers in this range.",
                    )
                    .color(palette.muted),
                );
                return;
            }
            // Zoom to the process lifetime within the range, with a small margin.
            let margin = ((row.last_seen_ms - row.first_seen_ms) / 20).max(60_000);
            let view_from = (row.first_seen_ms - margin).max(from_ms);
            let view_to = (row.last_seen_ms + margin).min(to_ms);
            let opts = |y_max, format_y| HistoryChartOptions {
                height: 110.0,
                from_secs: view_from as f64 / 1000.0,
                to_secs: view_to as f64 / 1000.0,
                y_max,
                format_y,
            };
            ui.columns(2, |cols| {
                history_chart(
                    &mut cols[0],
                    "hist_proc_cpu",
                    "history_proc",
                    &[HistorySeries {
                        name: "CPU".into(),
                        color: palette.cpu,
                        thin: false,
                        segments: split_segments(points.iter().map(|p| (p.ts_ms, p.cpu)), gap_ms),
                    }],
                    &opts(Some(100.0), |v| format!("{v:.0}%")),
                );
                history_chart(
                    &mut cols[1],
                    "hist_proc_mem",
                    "history_proc",
                    &[
                        HistorySeries {
                            name: "Working set".into(),
                            color: palette.memory,
                            thin: false,
                            segments: split_segments(
                                points.iter().map(|p| (p.ts_ms, p.working_set.map(|v| v as f64))),
                                gap_ms,
                            ),
                        },
                        HistorySeries {
                            name: "Private bytes".into(),
                            color: palette.accent,
                            thin: true,
                            segments: split_segments(
                                points
                                    .iter()
                                    .map(|p| (p.ts_ms, p.private_bytes.map(|v| v as f64))),
                                gap_ms,
                            ),
                        },
                    ],
                    &opts(None, |v| fmt::bytes(v.max(0.0) as u64)),
                );
            });
        });
    }
}
