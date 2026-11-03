//! One module per navigation section.

pub mod details;
pub mod history;
pub mod overview;
pub mod performance;
pub mod processes;
pub mod services;
pub mod settings;
pub mod startup;

use crate::core::models::Section;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageStatus {
    Ready,
    /// Not implemented yet; the value names the planned development phase.
    Planned(&'static str),
}

pub fn status(section: Section) -> PageStatus {
    match section {
        Section::Overview | Section::Settings | Section::Processes | Section::Details => PageStatus::Ready,
        Section::Performance => PageStatus::Planned("Phase 3"),
        Section::Services | Section::Startup => PageStatus::Planned("Phase 4"),
        Section::History => PageStatus::Planned("Phase 5"),
    }
}
