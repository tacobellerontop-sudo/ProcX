//! Windows implementation of the platform layer (via `windows-rs`).

mod handle;
mod system;

pub use system::{is_elevated, kernel_counters};
