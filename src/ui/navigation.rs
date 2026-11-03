//! Navigation and global keyboard shortcuts.

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::core::models::Section;

const SECTION_KEYS: [Key; 8] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
];

/// Ctrl+1 … Ctrl+8 select the sections in sidebar order.
pub fn section_shortcut(section: Section) -> KeyboardShortcut {
    let index = Section::ALL.iter().position(|&s| s == section).unwrap_or(0);
    KeyboardShortcut::new(Modifiers::COMMAND, SECTION_KEYS[index])
}

pub const REFRESH_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F5);
pub const HELP_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F1);

/// Every shortcut, for the help dialog: (keys, description).
pub fn all_shortcuts(ctx: &egui::Context) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = Section::ALL
        .iter()
        .map(|&s| {
            (
                ctx.format_shortcut(&section_shortcut(s)),
                format!("Go to {}", s.label()),
            )
        })
        .collect();
    list.extend(
        [
            ("F5", "Take a sample now"),
            ("F1", "Show keyboard shortcuts"),
            ("Ctrl+F", "Processes: focus the filter"),
            ("Ctrl+C", "Processes: copy the selected rows"),
            (
                "Delete",
                "Processes: end the selected processes (with confirmation)",
            ),
            ("Enter", "Processes: open details of the selected process"),
            (
                "Shift / Ctrl + click",
                "Processes: select a range / add to the selection",
            ),
            ("Double-click", "Charts on the History page: reset zoom"),
            ("Esc", "Close a dialog"),
        ]
        .map(|(k, d)| (k.to_owned(), d.to_owned())),
    );
    list
}

/// Actions triggered by global shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalAction {
    Navigate(Section),
    Refresh,
    Help,
}

/// Consumes global shortcuts for this frame.
pub fn handle_shortcuts(ctx: &egui::Context) -> Option<GlobalAction> {
    ctx.input_mut(|input| {
        if input.consume_shortcut(&REFRESH_SHORTCUT) {
            return Some(GlobalAction::Refresh);
        }
        if input.consume_shortcut(&HELP_SHORTCUT) {
            return Some(GlobalAction::Help);
        }
        Section::ALL
            .into_iter()
            .find(|&s| input.consume_shortcut(&section_shortcut(s)))
            .map(GlobalAction::Navigate)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_has_a_distinct_shortcut() {
        let keys: std::collections::HashSet<_> = Section::ALL
            .iter()
            .map(|&s| section_shortcut(s).logical_key)
            .collect();
        assert_eq!(keys.len(), Section::ALL.len());
    }
}
