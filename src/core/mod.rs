//! Platform-independent data models, configuration and shared types.

pub mod config;
pub mod errors;
pub mod events;
pub mod health;
pub mod models;
pub mod series;
// Constructed only by the Windows backend.
#[cfg_attr(not(windows), allow(dead_code))]
pub mod services;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod startup;
