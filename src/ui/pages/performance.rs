//! Real-time performance dashboard: CPU, memory, disks and network.

use egui::RichText;

use crate::app::live::{LiveState, retained_secs};
use crate::core::models::SystemSnapshot;
use crate::ui::components::charts::{ChartOptions, ChartSeries, live_chart};
use crate::ui::components::metrics::{headline, stat};
use crate::ui::components::{page_header, section};
use crate::ui::theme::Palette;
use crate::utils::formatting::{self as fmt, UNAVAILABLE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Cpu,
    Memory,
    Disk,
    Network,
}

const RANGES: [(u32, &str); 4] = [(60, "1 min"), (300, "5 min"), (900, "15 min"), (3_600, "1 hour")];

pub struct PerformancePage {
    tab: Tab,
    range_secs: u32,
}

impl PerformancePage {
    pub fn new(default_range_secs: u32) -> Self {
        let range_secs = RANGES
            .iter()
            .map(|&(r, _)| r)
            .find(|&r| r >= default_range_secs)
            .unwrap_or(RANGES[RANGES.len() - 1].0);
        Self {
            tab: Tab::Cpu,
            range_secs,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, live: &LiveState, interval_ms: u64) {
        page_header(ui, "Performance", None);
        let Some(s) = live.latest.as_deref() else {
            ui.label(RichText::new("Collecting the first sample…").color(Palette::of(ui).muted));
            return;
        };
        let retained = retained_secs(interval_ms);
        ui.horizontal(|ui| {
            for (tab, label) in [
                (Tab::Cpu, "CPU"),
                (Tab::Memory, "Memory"),
                (Tab::Disk, "Disk"),
                (Tab::Network, "Network"),
            ] {
                ui.selectable_value(&mut self.tab, tab, label);
            }
            ui.separator();
            ui.label(RichText::new("Range").color(Palette::of(ui).muted));
            for (secs, label) in RANGES {
                let available = f64::from(secs) <= retained + 1.0;
                let response = ui.add_enabled(
                    available,
                    egui::Button::selectable(self.range_secs == secs, label),
                );
                let response = if available {
                    response
                } else {
                    response.on_disabled_hover_text(format!(
                        "At a {interval_ms} ms refresh interval only {:.0} minutes are kept in memory",
                        retained / 60.0
                    ))
                };
                if response.clicked() {
                    self.range_secs = secs;
                }
            }
        });
        if f64::from(self.range_secs) > retained + 1.0 {
            self.range_secs = RANGES
                .iter()
                .rev()
                .map(|&(r, _)| r)
                .find(|&r| f64::from(r) <= retained + 1.0)
                .unwrap_or(60);
        }
        ui.add_space(6.0);
        let window = f64::from(self.range_secs);
        let now = live.chart_now();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| match self.tab {
                Tab::Cpu => cpu(ui, live, s, now, window),
                Tab::Memory => memory(ui, live, s, now, window),
                Tab::Disk => disk(ui, live, s, now, window),
                Tab::Network => network(ui, live, s, now, window),
            });
    }
}

fn pct(v: f64) -> String {
    format!("{v:.0}%")
}

fn rate(v: f64) -> String {
    fmt::rate(v)
}

fn bytes(v: f64) -> String {
    fmt::bytes(v.max(0.0) as u64)
}

#[allow(clippy::too_many_arguments)] // thin wrapper over ChartOptions
fn chart(
    ui: &mut egui::Ui,
    id: &str,
    now: f64,
    series: &[ChartSeries<'_>],
    height: f32,
    window: f64,
    y_max: Option<f64>,
    format_y: fn(f64) -> String,
) {
    live_chart(
        ui,
        id,
        now,
        series,
        &ChartOptions {
            height,
            window_secs: window,
            y_max,
            format_y,
        },
    );
}

fn cpu(ui: &mut egui::Ui, live: &LiveState, s: &SystemSnapshot, now: f64, window: f64) {
    let p = Palette::of(ui);
    let info = live.info.as_ref();
    section(ui, "CPU", |ui| {
        ui.horizontal(|ui| {
            headline(ui, &fmt::percent(s.cpu.total_usage), "utilization", p.cpu);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(info.and_then(|i| i.cpu_brand.clone()).unwrap_or_default()).color(p.muted),
                );
            });
        });
        chart(
            ui,
            "perf_cpu",
            now,
            &[ChartSeries {
                name: "CPU",
                data: &live.series.cpu,
                color: p.cpu,
            }],
            160.0,
            window,
            Some(100.0),
            pct,
        );
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 28.0;
            stat(
                ui,
                "Frequency",
                s.cpu
                    .frequency_mhz
                    .map(|f| format!("{:.2} GHz", f as f64 / 1000.0)),
                "Current processor frequency reported by the OS",
            );
            stat(
                ui,
                "Logical processors",
                Some(s.cpu.per_core.len().to_string()),
                "Hardware threads visible to Windows",
            );
            stat(
                ui,
                "Cores",
                info.and_then(|i| i.physical_cores).map(|c| c.to_string()),
                "Physical cores",
            );
            stat(
                ui,
                "Processes",
                Some(fmt::thousands(s.counts.processes as u64)),
                "Running processes",
            );
            stat(
                ui,
                "Threads",
                s.counts.threads.map(fmt::thousands),
                "Threads across all processes",
            );
            stat(
                ui,
                "Handles",
                s.counts.handles.map(fmt::thousands),
                "Open kernel object handles",
            );
            stat(ui, "Uptime", s.uptime.map(fmt::uptime), "Time since boot");
        });
    });
    ui.add_space(6.0);
    section(ui, "LOGICAL PROCESSORS", |ui| {
        let n = live.series.per_core.len();
        if n == 0 {
            ui.label(RichText::new("Per-processor utilization is unavailable").color(p.muted));
            return;
        }
        let cols = ((n as f64).sqrt().ceil() as usize).clamp(2, 8);
        for row_start in (0..n).step_by(cols) {
            ui.columns(cols, |columns| {
                for (offset, ui) in columns.iter_mut().enumerate() {
                    let i = row_start + offset;
                    let Some(series) = live.series.per_core.get(i) else {
                        continue;
                    };
                    let current = s.cpu.per_core.get(i).copied().unwrap_or(0.0);
                    ui.label(
                        RichText::new(format!("CPU {i}   {current:.0}%"))
                            .small()
                            .color(p.muted),
                    );
                    chart(
                        ui,
                        &format!("core{i}"),
                        now,
                        &[ChartSeries {
                            name: "CPU",
                            data: series,
                            color: p.cpu,
                        }],
                        56.0,
                        window,
                        Some(100.0),
                        pct,
                    );
                }
            });
            ui.add_space(4.0);
        }
    });
}

fn memory(ui: &mut egui::Ui, live: &LiveState, s: &SystemSnapshot, now: f64, window: f64) {
    let p = Palette::of(ui);
    let m = &s.memory;
    section(ui, "MEMORY", |ui| {
        headline(
            ui,
            &fmt::percent(m.used_percent()),
            &format!("{} of {} in use", fmt::bytes(m.used), fmt::bytes(m.total)),
            p.memory,
        );
        chart(
            ui,
            "perf_mem",
            now,
            &[ChartSeries {
                name: "In use",
                data: &live.series.memory,
                color: p.memory,
            }],
            160.0,
            window,
            Some(100.0),
            pct,
        );
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 28.0;
            stat(
                ui,
                "Total",
                Some(fmt::bytes(m.total)),
                "Installed physical memory usable by Windows",
            );
            stat(ui, "In use", Some(fmt::bytes(m.used)), "Physical memory in use");
            stat(
                ui,
                "Available",
                Some(fmt::bytes(m.available)),
                "Memory available to processes without paging",
            );
            stat(ui, "Cached", m.system_cache.map(fmt::bytes), "System file cache");
            stat(
                ui,
                "Paged pool",
                m.kernel_paged.map(fmt::bytes),
                "Kernel memory that can be paged out",
            );
            stat(
                ui,
                "Non-paged pool",
                m.kernel_nonpaged.map(fmt::bytes),
                "Kernel memory that must stay resident",
            );
        });
    });
    ui.add_space(6.0);
    section(ui, "COMMIT AND PAGE FILE", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 28.0;
            stat(
                ui,
                "Committed",
                m.commit_total
                    .zip(m.commit_limit)
                    .map(|(t, l)| format!("{} / {}", fmt::bytes(t), fmt::bytes(l))),
                "Commit charge / commit limit (RAM + page files)",
            );
            stat(
                ui,
                "Page file",
                (m.swap_total > 0)
                    .then(|| format!("{} / {}", fmt::bytes(m.swap_used), fmt::bytes(m.swap_total))),
                "Page file usage / size",
            );
        });
        if !live.series.commit.is_empty() {
            chart(
                ui,
                "perf_commit",
                now,
                &[ChartSeries {
                    name: "Committed",
                    data: &live.series.commit,
                    color: p.memory,
                }],
                110.0,
                window,
                m.commit_limit.map(|l| l as f64),
                bytes,
            );
        }
    });
}

fn disk(ui: &mut egui::Ui, live: &LiveState, s: &SystemSnapshot, now: f64, window: f64) {
    let p = Palette::of(ui);
    if s.physical_disks.is_empty() {
        section(ui, "ALL DISKS", |ui| {
            ui.label(RichText::new("Per-physical-disk activity is unavailable on this platform; showing combined volume throughput.").small().color(p.muted));
            chart(
                ui,
                "perf_disk_all",
                now,
                &[
                    ChartSeries {
                        name: "Read",
                        data: &live.series.disk_read,
                        color: p.disk_read,
                    },
                    ChartSeries {
                        name: "Write",
                        data: &live.series.disk_write,
                        color: p.disk_write,
                    },
                ],
                140.0,
                window,
                None,
                rate,
            );
        });
        ui.add_space(6.0);
    }
    for d in &s.physical_disks {
        let Some(series) = live.series.physical.get(&d.index) else {
            continue;
        };
        section(ui, &d.label().to_uppercase(), |ui| {
            ui.horizontal(|ui| {
                let active = d
                    .active_percent
                    .map_or_else(|| UNAVAILABLE.to_owned(), |a| format!("{a:.0}%"));
                headline(ui, &active, "active time", p.disk_read)
                    .on_hover_text("Share of time the disk was busy with requests (1 − idle time)");
                ui.label(
                    RichText::new(format!(
                        "Read {} · Write {}",
                        fmt::rate(d.read_bytes_per_sec),
                        fmt::rate(d.write_bytes_per_sec)
                    ))
                    .color(p.muted),
                );
            });
            ui.columns(2, |cols| {
                chart(
                    &mut cols[0],
                    &format!("pd_act{}", d.index),
                    now,
                    &[ChartSeries {
                        name: "Active",
                        data: &series.active,
                        color: p.disk_read,
                    }],
                    100.0,
                    window,
                    Some(100.0),
                    pct,
                );
                chart(
                    &mut cols[1],
                    &format!("pd_rw{}", d.index),
                    now,
                    &[
                        ChartSeries {
                            name: "Read",
                            data: &series.read,
                            color: p.disk_read,
                        },
                        ChartSeries {
                            name: "Write",
                            data: &series.write,
                            color: p.disk_write,
                        },
                    ],
                    100.0,
                    window,
                    None,
                    rate,
                );
            });
        });
        ui.add_space(6.0);
    }
    section(ui, "VOLUMES", |ui| {
        egui::Grid::new("volumes")
            .num_columns(7)
            .striped(true)
            .spacing([18.0, 4.0])
            .show(ui, |ui| {
                for h in [
                    "Volume",
                    "Device",
                    "Type",
                    "Capacity",
                    "Free",
                    "Used",
                    "Read / Write",
                ] {
                    ui.label(RichText::new(h).small().strong().color(p.muted));
                }
                ui.end_row();
                for v in &s.disks {
                    ui.label(&v.mount_point);
                    ui.label(&v.name);
                    ui.label(if v.removable {
                        format!("{} (removable)", v.kind.label())
                    } else {
                        v.kind.label().to_owned()
                    });
                    ui.monospace(fmt::bytes(v.total_space));
                    ui.monospace(fmt::bytes(v.available_space));
                    ui.monospace(fmt::percent(v.used_percent()));
                    ui.monospace(format!(
                        "{} / {}",
                        fmt::rate(v.read_bytes_per_sec),
                        fmt::rate(v.write_bytes_per_sec)
                    ));
                    ui.end_row();
                }
            });
    });
}

fn network(ui: &mut egui::Ui, live: &LiveState, s: &SystemSnapshot, now: f64, window: f64) {
    let p = Palette::of(ui);
    let n = &s.network;
    section(ui, "ALL INTERFACES", |ui| {
        headline(
            ui,
            &fmt::rate(n.received_per_sec + n.transmitted_per_sec),
            &format!(
                "Receive {} · Send {}",
                fmt::rate(n.received_per_sec),
                fmt::rate(n.transmitted_per_sec)
            ),
            p.net_rx,
        );
        chart(
            ui,
            "perf_net",
            now,
            &[
                ChartSeries {
                    name: "Receive",
                    data: &live.series.net_rx,
                    color: p.net_rx,
                },
                ChartSeries {
                    name: "Send",
                    data: &live.series.net_tx,
                    color: p.net_tx,
                },
            ],
            150.0,
            window,
            None,
            rate,
        );
    });
    for i in &n.interfaces {
        let Some(series) = live.series.interfaces.get(&i.name) else {
            continue;
        };
        ui.add_space(6.0);
        section(ui, &i.name, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 28.0;
                stat(
                    ui,
                    "Receive",
                    Some(fmt::rate(i.received_per_sec)),
                    "Current download throughput",
                );
                stat(
                    ui,
                    "Send",
                    Some(fmt::rate(i.transmitted_per_sec)),
                    "Current upload throughput",
                );
                stat(
                    ui,
                    "Total received",
                    Some(fmt::bytes(i.total_received)),
                    "Since the interface came up",
                );
                stat(
                    ui,
                    "Total sent",
                    Some(fmt::bytes(i.total_transmitted)),
                    "Since the interface came up",
                );
                stat(
                    ui,
                    "Link speed",
                    i.link_speed_bps.map(fmt::bits_per_sec),
                    "Negotiated link speed",
                );
                stat(ui, "MAC", i.mac_address.clone(), "Hardware address");
            });
            if !i.addresses.is_empty() {
                ui.label(
                    RichText::new(i.addresses.join("   "))
                        .small()
                        .monospace()
                        .color(p.muted),
                );
            }
            chart(
                ui,
                &format!("if_{}", i.name),
                now,
                &[
                    ChartSeries {
                        name: "Receive",
                        data: &series.rx,
                        color: p.net_rx,
                    },
                    ChartSeries {
                        name: "Send",
                        data: &series.tx,
                        color: p.net_tx,
                    },
                ],
                90.0,
                window,
                None,
                rate,
            );
        });
    }
}
