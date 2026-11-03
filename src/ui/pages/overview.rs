//! Overview dashboard: current system status at a glance.

use std::time::SystemTime;

use egui::RichText;

use crate::app::live::LiveState;
use crate::core::health::{self, CPU_SUSTAIN_SECS};
use crate::core::models::{ActivityKind, SystemSnapshot};
use crate::ui::components::charts::{ChartOptions, ChartSeries, live_chart};
use crate::ui::components::metrics::{headline, severity_label, stat, status_dot, usage_bar};
use crate::ui::components::tables::{TopMetric, top_processes};
use crate::ui::components::{page_header, section};
use crate::ui::theme::Palette;
use crate::utils::{formatting as fmt, time};

const CHART_HEIGHT: f32 = 72.0;

fn percent_axis(v: f64) -> String {
    format!("{v:.0}%")
}

fn rate_axis(v: f64) -> String {
    fmt::rate(v)
}

/// Splits the available width into `n` columns, falling back to fewer
/// columns when the window is narrow.
fn responsive_columns(
    ui: &mut egui::Ui,
    n: usize,
    min_width: f32,
    mut add: impl FnMut(usize, &mut egui::Ui),
) {
    let fit = ((ui.available_width() / min_width).floor() as usize).clamp(1, n);
    let mut index = 0;
    while index < n {
        let row = fit.min(n - index);
        ui.columns(row, |cols| {
            for col in cols.iter_mut() {
                add(index, col);
                index += 1;
            }
        });
        ui.add_space(6.0);
    }
}

pub fn show(ui: &mut egui::Ui, live: &LiveState, chart_window_secs: u32) {
    let subtitle = live.info.as_ref().map(|info| {
        [info.host_name.as_deref(), info.os_version.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
    });
    let header = page_header(ui, "Overview", subtitle.as_deref());
    if let Some(info) = &live.info {
        header.on_hover_text(format!(
            "Kernel {}\nArchitecture {}",
            info.kernel_version.as_deref().unwrap_or(fmt::UNAVAILABLE),
            info.cpu_arch
        ));
    }

    let Some(snapshot) = live.latest.as_deref() else {
        ui.label(RichText::new("Collecting the first sample…").color(Palette::of(ui).muted));
        return;
    };

    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let window = f64::from(chart_window_secs);
        responsive_columns(ui, 4, 240.0, |i, ui| match i {
            0 => cpu_panel(ui, live, snapshot, window),
            1 => memory_panel(ui, live, snapshot, window),
            2 => disk_panel(ui, live, snapshot, window),
            _ => network_panel(ui, live, snapshot, window),
        });
        counts_strip(ui, live, snapshot);
        ui.add_space(6.0);
        responsive_columns(ui, 3, 300.0, |i, ui| match i {
            0 => section(ui, "TOP CPU", |ui| {
                top_processes(
                    ui,
                    "top_cpu",
                    &snapshot.top_cpu,
                    TopMetric::Cpu,
                    snapshot.memory.total,
                )
            }),
            1 => section(ui, "TOP MEMORY", |ui| {
                top_processes(
                    ui,
                    "top_mem",
                    &snapshot.top_memory,
                    TopMetric::Memory,
                    snapshot.memory.total,
                )
            }),
            _ => {
                health_panel(ui, live, snapshot);
                ui.add_space(6.0);
                activity_panel(ui, live, snapshot.captured_at);
            }
        });
    });
}

fn cpu_panel(ui: &mut egui::Ui, live: &LiveState, s: &SystemSnapshot, window: f64) {
    let p = Palette::of(ui);
    section(ui, "CPU", |ui| {
        headline(ui, &fmt::percent(s.cpu.total_usage), "utilization", p.cpu);
        let freq = s.cpu.frequency_mhz.map_or_else(
            || fmt::UNAVAILABLE.to_owned(),
            |f| format!("{:.2} GHz", f as f64 / 1000.0),
        );
        let cores = live
            .info
            .as_ref()
            .and_then(|i| i.physical_cores)
            .map_or_else(String::new, |c| format!(" · {c} cores"));
        let logical = live
            .info
            .as_ref()
            .map_or(s.cpu.per_core.len(), |i| i.logical_cpus);
        ui.label(
            RichText::new(format!("{freq} · {logical} logical{cores}"))
                .small()
                .color(p.muted),
        )
        .on_hover_text(
            "Current processor frequency as reported by the OS, logical processors and physical cores",
        );
        live_chart(
            ui,
            "ov_cpu",
            live.chart_now(),
            &[ChartSeries {
                name: "CPU",
                data: &live.series.cpu,
                color: p.cpu,
            }],
            &ChartOptions {
                height: CHART_HEIGHT,
                window_secs: window,
                y_max: Some(100.0),
                format_y: percent_axis,
            },
        );
    });
}

fn memory_panel(ui: &mut egui::Ui, live: &LiveState, s: &SystemSnapshot, window: f64) {
    let p = Palette::of(ui);
    let m = &s.memory;
    section(ui, "MEMORY", |ui| {
        headline(
            ui,
            &fmt::percent(m.used_percent()),
            &format!("{} / {}", fmt::bytes(m.used), fmt::bytes(m.total)),
            p.memory,
        );
        ui.label(
            RichText::new(format!("Available {}", fmt::bytes(m.available)))
                .small()
                .color(p.muted),
        )
        .on_hover_text("Memory that can be given to processes without paging (standby + free)");
        live_chart(
            ui,
            "ov_mem",
            live.chart_now(),
            &[ChartSeries {
                name: "Memory",
                data: &live.series.memory,
                color: p.memory,
            }],
            &ChartOptions {
                height: CHART_HEIGHT,
                window_secs: window,
                y_max: Some(100.0),
                format_y: percent_axis,
            },
        );
    });
}

fn disk_panel(ui: &mut egui::Ui, live: &LiveState, s: &SystemSnapshot, window: f64) {
    let p = Palette::of(ui);
    let (read, write) = (s.disk_read_per_sec(), s.disk_write_per_sec());
    section(ui, "DISK", |ui| {
        let per_volume: String = s
            .disks
            .iter()
            .map(|d| {
                format!(
                    "\n{} {} ({}): read {}, write {}",
                    d.mount_point,
                    d.name,
                    d.kind.label(),
                    fmt::rate(d.read_bytes_per_sec),
                    fmt::rate(d.write_bytes_per_sec)
                )
            })
            .collect();
        let busiest = s
            .physical_disks
            .iter()
            .filter_map(|d| d.active_percent.map(|a| (d, a)))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        match busiest {
            Some((d, active)) => {
                headline(
                    ui,
                    &format!("{active:.0}%"),
                    &format!("active ({})", d.label()),
                    p.disk_read,
                )
                .on_hover_text(format!(
                    "Active time of the busiest physical disk. Throughput per volume:{per_volume}"
                ));
            }
            None => {
                headline(ui, &fmt::rate(read + write), "throughput", p.disk_read).on_hover_text(format!(
                    "Combined read + write throughput across all volumes. \
                     Disk active time is unavailable on this system.{per_volume}"
                ));
            }
        }
        ui.label(
            RichText::new(format!("Read {} · Write {}", fmt::rate(read), fmt::rate(write)))
                .small()
                .color(p.muted),
        );
        live_chart(
            ui,
            "ov_disk",
            live.chart_now(),
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
            &ChartOptions {
                height: CHART_HEIGHT,
                window_secs: window,
                y_max: None,
                format_y: rate_axis,
            },
        );
    });
}

fn network_panel(ui: &mut egui::Ui, live: &LiveState, s: &SystemSnapshot, window: f64) {
    let p = Palette::of(ui);
    let n = &s.network;
    section(ui, "NETWORK", |ui| {
        headline(
            ui,
            &fmt::rate(n.received_per_sec + n.transmitted_per_sec),
            "total",
            p.net_rx,
        );
        ui.label(
            RichText::new(format!(
                "Receive {} · Send {}",
                fmt::rate(n.received_per_sec),
                fmt::rate(n.transmitted_per_sec)
            ))
            .small()
            .color(p.muted),
        )
        .on_hover_text(
            n.interfaces
                .iter()
                .map(|i| {
                    format!(
                        "{}: receive {}, send {}",
                        i.name,
                        fmt::rate(i.received_per_sec),
                        fmt::rate(i.transmitted_per_sec)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        );
        live_chart(
            ui,
            "ov_net",
            live.chart_now(),
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
            &ChartOptions {
                height: CHART_HEIGHT,
                window_secs: window,
                y_max: None,
                format_y: rate_axis,
            },
        );
    });
}

fn counts_strip(ui: &mut egui::Ui, live: &LiveState, s: &SystemSnapshot) {
    section(ui, "SYSTEM", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 28.0;
            stat(ui, "Processes", Some(fmt::thousands(s.counts.processes as u64)), "Running processes");
            stat(
                ui,
                "Threads",
                s.counts.threads.map(fmt::thousands),
                "Threads across all processes (GetPerformanceInfo). Unavailable on this platform if shown as —",
            );
            stat(
                ui,
                "Handles",
                s.counts.handles.map(fmt::thousands),
                "Open kernel object handles across all processes (GetPerformanceInfo)",
            );
            stat(ui, "Uptime", s.uptime.map(fmt::uptime), "Time since the system booted");
            stat(
                ui,
                "Commit",
                s.memory
                    .commit_total
                    .zip(s.memory.commit_limit)
                    .map(|(t, l)| format!("{} / {}", fmt::bytes(t), fmt::bytes(l))),
                "Commit charge / commit limit: virtual memory backed by RAM and page files",
            );
            stat(
                ui,
                "Page file",
                (s.memory.swap_total > 0)
                    .then(|| format!("{} / {}", fmt::bytes(s.memory.swap_used), fmt::bytes(s.memory.swap_total))),
                "Page file usage / size",
            );
            if let Some(info) = &live.info {
                stat(ui, "CPU", info.cpu_brand.clone(), "Processor model");
            }
        });
        if let Some(disk) = s.system_disk() {
            ui.add_space(4.0);
            let used = disk.total_space.saturating_sub(disk.available_space);
            ui.label(
                RichText::new(format!(
                    "{} ({}) {} of {} used",
                    disk.mount_point,
                    disk.kind.label(),
                    fmt::bytes(used),
                    fmt::bytes(disk.total_space)
                ))
                .small()
                .color(Palette::of(ui).muted),
            );
            usage_bar(ui, disk.used_percent(), Palette::of(ui).disk_read);
        }
    });
}

fn health_panel(ui: &mut egui::Ui, live: &LiveState, s: &SystemSnapshot) {
    let p = Palette::of(ui);
    section(ui, "HEALTH", |ui| {
        for ind in health::evaluate(s, live.series.cpu.mean_over(CPU_SUSTAIN_SECS)) {
            status_dot(
                ui,
                p.severity(ind.severity),
                format!("{}: {}", ind.name, ind.detail),
            )
            .on_hover_text(severity_label(ind.severity));
        }
    });
}

fn activity_panel(ui: &mut egui::Ui, live: &LiveState, now: SystemTime) {
    let p = Palette::of(ui);
    section(ui, "RECENT ACTIVITY", |ui| {
        if live.activity.is_empty() {
            ui.label(RichText::new("No process starts or exits observed yet").color(p.muted));
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("activity")
            .max_height(180.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for e in &live.activity {
                    let (sign, color, verb) = match e.kind {
                        ActivityKind::ProcessStarted => ("+", p.ok, "started"),
                        ActivityKind::ProcessExited => ("−", p.muted, "exited"),
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(sign).monospace().color(color));
                        ui.label(format!("{} {verb}", e.name));
                        ui.label(RichText::new(format!("PID {}", e.key.pid)).small().color(p.muted));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(time::relative(e.at, now)).small().color(p.muted));
                        });
                    });
                }
            });
        if live.activity_dropped > 0 {
            ui.label(
                RichText::new(format!(
                    "{} more events not shown (burst limit)",
                    live.activity_dropped
                ))
                .small()
                .color(p.muted),
            );
        }
    });
}
