//! Virtualized, sortable, resizable process table (flat or tree).

use egui::{Align, Layout, RichText, Sense};
use egui_extras::{Column, TableBuilder};

use crate::core::models::{AccessLevel, ProcessInfo, ProcessKey, percent};
use crate::process::explorer::{ProcessColumn, Selection, SortSpec};
use crate::process::tree::TreeRow;
use crate::ui::theme::Palette;
use crate::utils::formatting::{self as fmt, UNAVAILABLE};

pub const ROW_HEIGHT: f32 = 20.0;
const INDENT: f32 = 14.0;

/// What the user did with the table this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum TableEvent {
    Sort(ProcessColumn),
    Click {
        key: ProcessKey,
        ctrl: bool,
        shift: bool,
    },
    /// Right-click on a row not yet selected selects it first.
    ContextSelect(ProcessKey),
    Open(ProcessKey),
    ToggleExpand(ProcessKey),
    Menu(MenuAction, ProcessKey),
}

/// Context menu commands. They apply to the clicked process, or to the whole
/// selection where that makes sense (copy, end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Properties,
    OpenLocation,
    CopyName,
    CopyPid,
    CopyPath,
    CopySelection,
    GoToParent,
    SelectChildren,
    Refresh,
    End,
}

fn context_menu(
    ui: &mut egui::Ui,
    p: &ProcessInfo,
    row: &TreeRow,
    selected: usize,
    events: &mut Vec<TableEvent>,
) {
    let mut item = |ui: &mut egui::Ui, enabled: bool, label: &str, action: MenuAction| {
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            events.push(TableEvent::Menu(action, p.key));
            ui.close();
        }
    };
    item(ui, true, "Properties", MenuAction::Properties);
    item(
        ui,
        p.exe_path.is_some(),
        "Open file location",
        MenuAction::OpenLocation,
    );
    ui.menu_button("Copy", |ui| {
        item(ui, true, "Name", MenuAction::CopyName);
        item(ui, true, "PID", MenuAction::CopyPid);
        item(ui, p.exe_path.is_some(), "Path", MenuAction::CopyPath);
        item(
            ui,
            true,
            "Selected rows (tab-separated)",
            MenuAction::CopySelection,
        );
    });
    ui.separator();
    item(
        ui,
        p.parent_pid.is_some() && !row.orphan,
        "Go to parent process",
        MenuAction::GoToParent,
    );
    item(ui, true, "Select child processes", MenuAction::SelectChildren);
    item(ui, true, "Refresh", MenuAction::Refresh);
    ui.separator();
    let label = if selected > 1 {
        format!("End {selected} processes…")
    } else {
        "End process…".to_owned()
    };
    let red = Palette::of(ui).critical;
    if ui.button(RichText::new(label).color(red)).clicked() {
        events.push(TableEvent::Menu(MenuAction::End, p.key));
        ui.close();
    }
}

pub struct TableInput<'a> {
    pub processes: &'a [ProcessInfo],
    pub rows: &'a [TreeRow],
    pub columns: &'a [ProcessColumn],
    pub sort: SortSpec,
    pub selection: &'a Selection,
    pub collapsed: &'a std::collections::HashSet<ProcessKey>,
    pub tree: bool,
    pub total_memory: u64,
    pub scroll_to_row: Option<usize>,
}

fn text_cell(ui: &mut egui::Ui, value: Option<String>, numeric: bool) {
    let layout = if numeric {
        Layout::right_to_left(Align::Center)
    } else {
        Layout::left_to_right(Align::Center)
    };
    ui.with_layout(layout, |ui| match value {
        Some(v) => {
            ui.add(egui::Label::new(RichText::new(v).monospace()).truncate());
        }
        None => {
            ui.label(RichText::new(UNAVAILABLE).color(Palette::of(ui).muted))
                .on_hover_text("Unavailable: not provided by Windows or access denied");
        }
    });
}

#[derive(Clone, Copy)]
enum Arrow {
    Right,
    Down,
    Up,
}

/// Paints a small triangle. Painted rather than drawn as a glyph so it does
/// not depend on font coverage.
fn arrow(ui: &mut egui::Ui, dir: Arrow, color: egui::Color32, sense: Sense) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), sense);
    let c = rect.center();
    let r = 3.5;
    let points = match dir {
        Arrow::Right => vec![
            c + egui::vec2(-r * 0.6, -r),
            c + egui::vec2(-r * 0.6, r),
            c + egui::vec2(r, 0.0),
        ],
        Arrow::Down => vec![
            c + egui::vec2(-r, -r * 0.6),
            c + egui::vec2(r, -r * 0.6),
            c + egui::vec2(0.0, r),
        ],
        Arrow::Up => vec![
            c + egui::vec2(-r, r * 0.6),
            c + egui::vec2(r, r * 0.6),
            c + egui::vec2(0.0, -r),
        ],
    };
    let color = if response.hovered() {
        ui.visuals().strong_text_color()
    } else {
        color
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(points, color, egui::Stroke::NONE));
    response
}

pub fn cell_value(p: &ProcessInfo, column: ProcessColumn, total_memory: u64) -> Option<String> {
    use ProcessColumn as C;
    match column {
        C::Name => Some(p.name.to_string()),
        C::Pid => Some(p.key.pid.to_string()),
        C::ParentPid => p.parent_pid.map(|v| v.to_string()),
        C::Cpu => p.cpu_percent.map(fmt::percent),
        C::Memory => p.working_set.map(fmt::bytes),
        C::MemoryPercent => p.working_set.map(|w| fmt::percent(percent(w, total_memory))),
        C::PrivateBytes => p.private_bytes.map(fmt::bytes),
        C::Threads => p.threads.map(|v| fmt::thousands(v.into())),
        C::Handles => p.handles.map(|v| fmt::thousands(v.into())),
        C::User => p.user.as_deref().map(str::to_owned),
        C::Architecture => p.architecture.map(|a| a.label()),
        C::Session => p.session_id.map(|v| v.to_string()),
        C::Status => Some(p.status.label().to_owned()),
        C::Path => p.exe_path.as_deref().map(str::to_owned),
    }
}

fn name_cell(
    ui: &mut egui::Ui,
    p: &ProcessInfo,
    row: &TreeRow,
    input: &TableInput<'_>,
    events: &mut Vec<TableEvent>,
) {
    let palette = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if input.tree {
            ui.add_space(row.depth as f32 * INDENT);
            if row.has_children {
                let collapsed = input.collapsed.contains(&p.key);
                let dir = if collapsed { Arrow::Right } else { Arrow::Down };
                if arrow(ui, dir, palette.muted, Sense::click())
                    .on_hover_text(if collapsed { "Expand" } else { "Collapse" })
                    .clicked()
                {
                    events.push(TableEvent::ToggleExpand(p.key));
                }
            } else {
                ui.add_space(10.0);
            }
        }
        let mut text = RichText::new(&*p.name);
        if input.tree && row.has_children {
            text = text.strong();
        }
        if p.access == AccessLevel::Denied {
            text = text.color(palette.muted);
        }
        let response = ui.add(egui::Label::new(text).truncate().selectable(false));
        let mut tip = String::new();
        if row.orphan
            && let Some(ppid) = p.parent_pid
        {
            tip.push_str(&format!("Parent process (PID {ppid}) has exited.\n"));
        }
        if p.access == AccessLevel::Denied {
            tip.push_str("Limited information: Windows denied access to this process.\n");
        }
        if let Some(path) = &p.exe_path {
            tip.push_str(path);
        }
        if !tip.is_empty() {
            response.on_hover_text(tip.trim_end());
        }
    });
}

pub fn show(ui: &mut egui::Ui, input: &TableInput<'_>) -> Vec<TableEvent> {
    let mut events = Vec::new();

    let total_width: f32 = input.columns.iter().map(|c| c.initial_width()).sum::<f32>() + 40.0;
    egui::ScrollArea::horizontal()
        .id_salt("process_table_h")
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.set_min_width(total_width.min(ui.available_width().max(total_width)));
            table(ui, input, &mut events);
        });
    events
}

fn table(ui: &mut egui::Ui, input: &TableInput<'_>, events: &mut Vec<TableEvent>) {
    let palette = Palette::of(ui);
    let mut builder = TableBuilder::new(ui)
        .id_salt("process_table")
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .auto_shrink(false);
    for (i, &column) in input.columns.iter().enumerate() {
        let c = Column::initial(column.initial_width()).at_least(36.0).clip(true);
        builder = builder.column(if i + 1 == input.columns.len() {
            c.at_least(80.0)
        } else {
            c
        });
    }
    if let Some(row) = input.scroll_to_row {
        builder = builder.scroll_to_row(row, Some(Align::Center));
    }

    builder
        .header(ROW_HEIGHT + 2.0, |mut header| {
            for &column in input.columns {
                header.col(|ui| {
                    let sorted = input.sort.column == column;
                    let text = RichText::new(column.label()).small().strong();
                    let text = if sorted { text.color(palette.accent) } else { text };
                    let label = ui
                        .add(egui::Label::new(text).sense(Sense::click()).selectable(false))
                        .on_hover_text(column.tooltip());
                    if sorted {
                        let dir = if input.sort.descending {
                            Arrow::Down
                        } else {
                            Arrow::Up
                        };
                        arrow(ui, dir, palette.accent, Sense::hover());
                    }
                    if label.clicked() {
                        events.push(TableEvent::Sort(column));
                    }
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, input.rows.len(), |mut table_row| {
                let row = input.rows[table_row.index()];
                let p = &input.processes[row.index];
                table_row.set_selected(input.selection.contains(&p.key));
                for &column in input.columns {
                    table_row.col(|ui| {
                        if column == ProcessColumn::Name {
                            name_cell(ui, p, &row, input, events);
                        } else {
                            text_cell(ui, cell_value(p, column, input.total_memory), column.is_numeric());
                        }
                    });
                }
                let response = table_row.response();
                if response.double_clicked() {
                    events.push(TableEvent::Open(p.key));
                } else if response.clicked() {
                    let m = response.ctx.input(|i| i.modifiers);
                    events.push(TableEvent::Click {
                        key: p.key,
                        ctrl: m.command,
                        shift: m.shift,
                    });
                } else if response.secondary_clicked() && !input.selection.contains(&p.key) {
                    events.push(TableEvent::ContextSelect(p.key));
                }
                let selected = if input.selection.contains(&p.key) {
                    input.selection.len()
                } else {
                    1
                };
                response.context_menu(|ui| context_menu(ui, p, &row, selected, events));
            });
        });
}
