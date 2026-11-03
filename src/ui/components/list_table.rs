//! A sortable, virtualized single-selection table used by the Services,
//! Startup and History pages.

use egui::{Align, Layout, RichText, Sense};
use egui_extras::{Column, TableBuilder};

use crate::ui::components::process_table::{Arrow, arrow};
use crate::ui::theme::Palette;

pub struct ListColumn {
    pub label: &'static str,
    pub width: f32,
    pub tooltip: &'static str,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ListSort {
    pub column: usize,
    pub descending: bool,
}

impl ListSort {
    /// Clicking the sorted column flips direction; another column sorts ascending.
    pub fn toggle(&mut self, column: usize) {
        if self.column == column {
            self.descending = !self.descending;
        } else {
            self.column = column;
            self.descending = false;
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ListEvents {
    pub sort: Option<usize>,
    pub clicked: Option<usize>,
    pub double_clicked: Option<usize>,
}

/// One table instance's parameters.
pub struct ListTable<'a> {
    pub id: &'a str,
    pub columns: &'a [ListColumn],
    pub sort: ListSort,
    pub rows: usize,
    pub selected: Option<usize>,
    /// Caps the table so content below it stays visible.
    pub height: f32,
}

/// Shows the table; `cell(ui, row, column)` draws one cell.
pub fn show(
    ui: &mut egui::Ui,
    table: ListTable<'_>,
    mut cell: impl FnMut(&mut egui::Ui, usize, usize),
) -> ListEvents {
    let ListTable {
        id,
        columns,
        sort,
        rows,
        selected,
        height,
    } = table;
    // A bounded child area keeps the column resize lines from running past
    // the table to the bottom of the page.
    let size = egui::vec2(ui.available_width(), height + HEADER_HEIGHT);
    ui.allocate_ui(size, |ui| {
        body(ui, id, columns, sort, rows, selected, height, &mut cell)
    })
    .inner
}

const HEADER_HEIGHT: f32 = 22.0;

#[allow(clippy::too_many_arguments)]
fn body(
    ui: &mut egui::Ui,
    id: &str,
    columns: &[ListColumn],
    sort: ListSort,
    rows: usize,
    selected: Option<usize>,
    height: f32,
    cell: &mut impl FnMut(&mut egui::Ui, usize, usize),
) -> ListEvents {
    let palette = Palette::of(ui);
    let mut events = ListEvents::default();
    let mut builder = TableBuilder::new(ui)
        .id_salt(id)
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .min_scrolled_height(0.0)
        .max_scroll_height(height)
        .cell_layout(Layout::left_to_right(Align::Center))
        .auto_shrink([false, true]);
    for (i, c) in columns.iter().enumerate() {
        let col = if i + 1 == columns.len() {
            Column::remainder().at_least(80.0).clip(true)
        } else {
            Column::initial(c.width).at_least(40.0).clip(true)
        };
        builder = builder.column(col);
    }
    builder
        .header(HEADER_HEIGHT, |mut header| {
            for (i, c) in columns.iter().enumerate() {
                header.col(|ui| {
                    let sorted = sort.column == i;
                    let mut text = RichText::new(c.label).small().strong();
                    if sorted {
                        text = text.color(palette.accent);
                    }
                    let mut response = ui.add(egui::Label::new(text).sense(Sense::click()).selectable(false));
                    if !c.tooltip.is_empty() {
                        response = response.on_hover_text(c.tooltip);
                    }
                    if sorted {
                        let dir = if sort.descending { Arrow::Down } else { Arrow::Up };
                        arrow(ui, dir, palette.accent, Sense::hover());
                    }
                    if response.clicked() {
                        events.sort = Some(i);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(20.0, rows, |mut row| {
                let index = row.index();
                row.set_selected(selected == Some(index));
                for column in 0..columns.len() {
                    row.col(|ui| cell(ui, index, column));
                }
                let response = row.response();
                if response.double_clicked() {
                    events.double_clicked = Some(index);
                } else if response.clicked() {
                    events.clicked = Some(index);
                }
            });
        });
    events
}

/// A truncated label with the full text on hover; `None` shows as unavailable.
pub fn text(ui: &mut egui::Ui, value: Option<&str>) {
    match value {
        Some(v) => {
            ui.add(egui::Label::new(v).truncate());
        }
        None => {
            ui.label(RichText::new(crate::utils::formatting::UNAVAILABLE).color(Palette::of(ui).muted));
        }
    }
}
