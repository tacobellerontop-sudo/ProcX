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

/// Actions triggered by global shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalAction {
    Navigate(Section),
    Refresh,
}

/// Consumes global shortcuts for this frame.
pub fn handle_shortcuts(ctx: &egui::Context) -> Option<GlobalAction> {
    ctx.input_mut(|input| {
        if input.consume_shortcut(&REFRESH_SHORTCUT) {
            return Some(GlobalAction::Refresh);
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
