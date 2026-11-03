//! Table helpers.

use egui::{Grid, RichText};

use crate::core::models::ProcessSummary;
use crate::ui::theme::Palette;
use crate::utils::formatting;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopMetric {
    Cpu,
    Memory,
}

/// Compact top-N process table. `total_memory` converts bytes to a share.
pub fn top_processes(
    ui: &mut egui::Ui,
    id: &str,
    rows: &[ProcessSummary],
    metric: TopMetric,
    total_memory: u64,
) {
    let muted = Palette::of(ui).muted;
    if rows.is_empty() {
        ui.label(RichText::new("No data yet").color(muted));
        return;
    }
    Grid::new(id)
        .num_columns(4)
        .striped(true)
        .spacing([12.0, 3.0])
        .min_col_width(40.0)
        .show(ui, |ui| {
            for h in ["Name", "PID", "CPU", "Memory"] {
                ui.label(RichText::new(h).small().color(muted));
            }
            ui.end_row();
            for row in rows {
                ui.label(&row.name)
                    .on_hover_text(format!("{} (PID {})", row.name, row.key.pid));
                ui.label(RichText::new(row.key.pid.to_string()).monospace().color(muted));
                let cpu = RichText::new(formatting::percent(row.cpu_percent)).monospace();
                let mem = RichText::new(formatting::bytes(row.memory_bytes)).monospace();
                match metric {
                    TopMetric::Cpu => {
                        ui.label(cpu.strong());
                        ui.label(mem);
                    }
                    TopMetric::Memory => {
                        ui.label(cpu);
                        ui.label(mem.strong()).on_hover_text(format!(
                            "{:.1}% of physical memory",
                            crate::core::models::percent(row.memory_bytes, total_memory)
                        ));
                    }
                }
                ui.end_row();
            }
        });
}

/// Two-column label/value grid with selectable values.
pub fn key_values(ui: &mut egui::Ui, id: &str, rows: &[(&str, String)]) {
    let muted = Palette::of(ui).muted;
    Grid::new(id).num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
        for (k, v) in rows {
            ui.label(RichText::new(*k).color(muted));
            ui.add(egui::Label::new(v).wrap().selectable(true));
            ui.end_row();
        }
    });
}
