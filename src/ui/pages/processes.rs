//! Process explorer page.

use std::collections::HashSet;
use std::sync::Arc;

use egui::{Key, KeyboardShortcut, Modifiers, RichText};

use crate::core::config::ProcessesConfig;
use crate::core::models::{ProcessInfo, ProcessKey};
use crate::process::explorer::{Filter, ProcessColumn, Selection, SortSpec, compare};
use crate::process::safety::check_terminate;
use crate::process::tree::{self, TreeRow};
use crate::ui::components::dialogs::{Confirm, ConfirmResult};
use crate::ui::components::page_header;
use crate::ui::components::process_table::{self, MenuAction, TableEvent, TableInput, cell_value};
use crate::ui::theme::Palette;

/// Requests the page makes of the application.
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessesAction {
    Terminate(Vec<(ProcessKey, String)>),
    OpenLocation(String),
    ShowDetails(ProcessKey),
    SaveConfig(ProcessesConfig),
    Refresh,
    Notice(String),
}

/// Cache key for the computed row order.
#[derive(PartialEq)]
struct RowsKey {
    data: usize,
    sort: SortSpec,
    filter: Filter,
    tree: bool,
    collapsed_generation: u64,
}

#[derive(Default)]
pub struct ProcessesPage {
    query: String,
    filter: Filter,
    sort: SortSpec,
    selection: Selection,
    collapsed: HashSet<ProcessKey>,
    collapsed_generation: u64,
    rows: Vec<TreeRow>,
    rows_key: Option<RowsKey>,
    pending_end: Option<Vec<(ProcessKey, String)>>,
    blocked: Option<String>,
    focus_filter: bool,
    scroll_to: Option<ProcessKey>,
}

const FIND: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
const COPY: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::C);

fn tsv_header(columns: &[ProcessColumn]) -> String {
    columns.iter().map(|c| c.label()).collect::<Vec<_>>().join("\t")
}

fn tsv_row(p: &ProcessInfo, columns: &[ProcessColumn], total_memory: u64) -> String {
    columns
        .iter()
        .map(|&c| cell_value(p, c, total_memory).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\t")
}

impl ProcessesPage {
    /// The primary selected process, used by the Details page.
    pub fn primary_selection(&self) -> Option<ProcessKey> {
        self.selection.primary()
    }

    pub fn select(&mut self, key: ProcessKey) {
        self.selection.set_single(key);
        self.scroll_to = Some(key);
    }

    fn refresh_rows(&mut self, processes: &Arc<[ProcessInfo]>, tree_view: bool) {
        let key = RowsKey {
            data: Arc::as_ptr(processes).cast::<u8>() as usize,
            sort: self.sort,
            filter: self.filter.clone(),
            tree: tree_view,
            collapsed_generation: self.collapsed_generation,
        };
        if self.rows_key.as_ref() == Some(&key) {
            return;
        }
        self.rows = if tree_view {
            tree::build(processes, self.sort, &self.filter, &self.collapsed)
        } else {
            let mut idx: Vec<usize> = (0..processes.len())
                .filter(|&i| self.filter.matches(&processes[i]))
                .collect();
            idx.sort_by(|&a, &b| compare(&processes[a], &processes[b], self.sort));
            idx.into_iter()
                .map(|index| TreeRow {
                    index,
                    depth: 0,
                    has_children: false,
                    orphan: false,
                })
                .collect()
        };
        let existing: HashSet<ProcessKey> = processes.iter().map(|p| p.key).collect();
        self.selection.retain_existing(&existing);
        self.collapsed.retain(|k| existing.contains(k));
        self.rows_key = Some(key);
    }

    fn selected_targets<'a>(&self, processes: &'a [ProcessInfo]) -> Vec<&'a ProcessInfo> {
        processes
            .iter()
            .filter(|p| self.selection.contains(&p.key))
            .collect()
    }

    /// Starts the end-process flow for the current selection.
    fn request_end(
        &mut self,
        processes: &[ProcessInfo],
        confirm: bool,
        own_pid: u32,
        actions: &mut Vec<ProcessesAction>,
    ) {
        let targets = self.selected_targets(processes);
        if targets.is_empty() {
            return;
        }
        let blocked: Vec<String> = targets
            .iter()
            .filter_map(|p| {
                check_terminate(p, own_pid)
                    .err()
                    .map(|b| format!("{} (PID {}): {}", p.name, p.key.pid, b.reason()))
            })
            .collect();
        if !blocked.is_empty() {
            self.blocked = Some(blocked.join("\n\n"));
            return;
        }
        let list: Vec<(ProcessKey, String)> = targets.iter().map(|p| (p.key, p.name.to_string())).collect();
        if confirm {
            self.pending_end = Some(list);
        } else {
            actions.push(ProcessesAction::Terminate(list));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        processes: Option<&Arc<[ProcessInfo]>>,
        total_memory: u64,
        config: &ProcessesConfig,
        confirm_end: bool,
        own_pid: u32,
    ) -> Vec<ProcessesAction> {
        let mut actions = Vec::new();
        page_header(ui, "Processes", None);
        let Some(processes) = processes else {
            ui.label(RichText::new("Collecting the first sample…").color(Palette::of(ui).muted));
            return actions;
        };
        self.refresh_rows(processes, config.tree_view);
        let palette = Palette::of(ui);

        // Toolbar.
        ui.horizontal(|ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Filter by name, path or PID (pid:1234)")
                    .desired_width(300.0),
            );
            if std::mem::take(&mut self.focus_filter) {
                edit.request_focus();
            }
            if edit.changed() {
                self.filter = Filter::parse(&self.query);
            }
            if !self.query.is_empty() && ui.small_button("✕").on_hover_text("Clear filter").clicked() {
                self.query.clear();
                self.filter = Filter::default();
            }
            ui.separator();
            let mut tree_view = config.tree_view;
            if ui
                .toggle_value(&mut tree_view, "Tree")
                .on_hover_text("Show parent/child relationships")
                .changed()
            {
                actions.push(ProcessesAction::SaveConfig(ProcessesConfig {
                    tree_view,
                    ..config.clone()
                }));
            }
            ui.menu_button("Columns", |ui| {
                for column in ProcessColumn::ALL {
                    let mut on = config.visible_columns.contains(&column);
                    let enabled = column != ProcessColumn::Name;
                    if ui
                        .add_enabled(enabled, egui::Checkbox::new(&mut on, column.label()))
                        .changed()
                    {
                        // Keep the canonical column order.
                        let visible = ProcessColumn::ALL
                            .into_iter()
                            .filter(|c| {
                                if *c == column {
                                    on
                                } else {
                                    config.visible_columns.contains(c)
                                }
                            })
                            .collect();
                        actions.push(ProcessesAction::SaveConfig(ProcessesConfig {
                            visible_columns: visible,
                            ..config.clone()
                        }));
                    }
                }
            });
            if ui.button("Refresh").on_hover_text("Sample now (F5)").clicked() {
                actions.push(ProcessesAction::Refresh);
            }
            let end = ui
                .add_enabled(
                    !self.selection.is_empty(),
                    egui::Button::new(RichText::new("End process…").color(palette.critical)),
                )
                .on_hover_text("End the selected processes (Delete)");
            if end.clicked() {
                self.request_end(processes, confirm_end, own_pid, &mut actions);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} processes · {} shown · {} selected",
                        processes.len(),
                        self.rows.len(),
                        self.selection.len()
                    ))
                    .small()
                    .color(palette.muted),
                );
            });
        });
        ui.add_space(4.0);

        // Keyboard shortcuts (only while no text field has focus).
        let typing = ui.ctx().egui_wants_keyboard_input();
        let (find, copy, delete, enter) = ui.input_mut(|i| {
            (
                i.consume_shortcut(&FIND),
                !typing && i.consume_shortcut(&COPY),
                !typing && i.consume_key(Modifiers::NONE, Key::Delete),
                !typing && i.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        if find {
            self.focus_filter = true;
        }
        if copy && !self.selection.is_empty() {
            self.copy_selection(ui.ctx(), processes, &config.visible_columns, total_memory);
        }
        if delete {
            self.request_end(processes, confirm_end, own_pid, &mut actions);
        }
        if enter && let Some(key) = self.selection.primary() {
            actions.push(ProcessesAction::ShowDetails(key));
        }

        let scroll_to_row = self
            .scroll_to
            .take()
            .and_then(|key| self.rows.iter().position(|r| processes[r.index].key == key));
        let input = TableInput {
            processes,
            rows: &self.rows,
            columns: &config.visible_columns,
            sort: self.sort,
            selection: &self.selection,
            collapsed: &self.collapsed,
            tree: config.tree_view,
            total_memory,
            scroll_to_row,
        };
        let events = process_table::show(ui, &input);

        for event in events {
            match event {
                TableEvent::Sort(column) => self.sort.clicked(column),
                TableEvent::Click { key, ctrl, shift } => {
                    let order: Vec<ProcessKey> = self.rows.iter().map(|r| processes[r.index].key).collect();
                    self.selection.click(key, ctrl, shift, &order);
                }
                TableEvent::ContextSelect(key) => self.selection.set_single(key),
                TableEvent::Open(key) => {
                    self.selection.set_single(key);
                    actions.push(ProcessesAction::ShowDetails(key));
                }
                TableEvent::ToggleExpand(key) => {
                    if !self.collapsed.remove(&key) {
                        self.collapsed.insert(key);
                    }
                    self.collapsed_generation += 1;
                }
                TableEvent::Menu(action, key) => {
                    self.menu_action(
                        ui.ctx(),
                        action,
                        key,
                        processes,
                        config,
                        total_memory,
                        confirm_end,
                        own_pid,
                        &mut actions,
                    );
                }
            }
        }

        self.dialogs(ui.ctx(), &mut actions);
        actions
    }

    fn copy_selection(
        &self,
        ctx: &egui::Context,
        processes: &[ProcessInfo],
        columns: &[ProcessColumn],
        total_memory: u64,
    ) {
        let mut text = tsv_header(columns);
        for row in &self.rows {
            let p = &processes[row.index];
            if self.selection.contains(&p.key) {
                text.push('\n');
                text.push_str(&tsv_row(p, columns, total_memory));
            }
        }
        ctx.copy_text(text);
    }

    #[allow(clippy::too_many_arguments)]
    fn menu_action(
        &mut self,
        ctx: &egui::Context,
        action: MenuAction,
        key: ProcessKey,
        processes: &[ProcessInfo],
        config: &ProcessesConfig,
        total_memory: u64,
        confirm_end: bool,
        own_pid: u32,
        actions: &mut Vec<ProcessesAction>,
    ) {
        let Some(p) = processes.iter().find(|p| p.key == key) else {
            actions.push(ProcessesAction::Notice("The process has exited".into()));
            return;
        };
        match action {
            MenuAction::Properties => actions.push(ProcessesAction::ShowDetails(key)),
            MenuAction::OpenLocation => {
                if let Some(path) = &p.exe_path {
                    actions.push(ProcessesAction::OpenLocation(path.to_string()));
                }
            }
            MenuAction::CopyName => ctx.copy_text(p.name.to_string()),
            MenuAction::CopyPid => ctx.copy_text(p.key.pid.to_string()),
            MenuAction::CopyPath => ctx.copy_text(p.exe_path.as_deref().unwrap_or_default().to_owned()),
            MenuAction::CopySelection => {
                self.copy_selection(ctx, processes, &config.visible_columns, total_memory)
            }
            MenuAction::GoToParent => {
                let parents = tree::parent_indices(processes);
                let index = processes.iter().position(|x| x.key == key);
                match index.and_then(|i| parents[i]) {
                    Some(parent) => {
                        let parent_key = processes[parent].key;
                        if !self.filter.is_empty() && !self.filter.matches(&processes[parent]) {
                            self.query.clear();
                            self.filter = Filter::default();
                        }
                        self.select(parent_key);
                    }
                    None => actions.push(ProcessesAction::Notice("The parent process has exited".into())),
                }
            }
            MenuAction::SelectChildren => {
                let children = tree::descendants(processes, key);
                if children.is_empty() {
                    actions.push(ProcessesAction::Notice(format!(
                        "{} has no child processes",
                        p.name
                    )));
                } else {
                    self.selection.set_many(children);
                }
            }
            MenuAction::Refresh => actions.push(ProcessesAction::Refresh),
            MenuAction::End => self.request_end(processes, confirm_end, own_pid, actions),
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context, actions: &mut Vec<ProcessesAction>) {
        if let Some(message) = &self.blocked {
            let dialog = Confirm {
                id: "end_blocked",
                title: "Cannot end process".into(),
                message: message.clone(),
                confirm_label: "OK".into(),
                destructive: false,
            };
            if dialog.show(ctx) != ConfirmResult::Pending {
                self.blocked = None;
            }
        }
        if let Some(targets) = &self.pending_end {
            let names: Vec<String> = targets
                .iter()
                .take(8)
                .map(|(k, n)| format!("{n} (PID {})", k.pid))
                .collect();
            let more = targets.len().saturating_sub(names.len());
            let mut message = format!(
                "{}\n\nUnsaved data in these processes will be lost. Ending a system or service process may make Windows unstable.",
                names.join("\n")
            );
            if more > 0 {
                message.insert_str(0, &format!("and {more} more:\n"));
            }
            let dialog = Confirm {
                id: "confirm_end",
                title: if targets.len() == 1 {
                    "End this process?".into()
                } else {
                    format!("End {} processes?", targets.len())
                },
                message,
                confirm_label: "End process".into(),
                destructive: true,
            };
            match dialog.show(ctx) {
                ConfirmResult::Pending => {}
                ConfirmResult::Cancelled => self.pending_end = None,
                ConfirmResult::Confirmed => {
                    if let Some(list) = self.pending_end.take() {
                        actions.push(ProcessesAction::Terminate(list));
                    }
                }
            }
        }
    }
}
