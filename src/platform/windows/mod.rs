//! Windows implementation of the platform layer (via `windows-rs`).

mod disk;
mod handle;
mod inspect;
mod processes;
mod system;
mod util;

pub use disk::physical_disk_counters;
pub use inspect::{inspect_process, open_file_location, terminate_process};
pub use processes::ProcessEnumerator;
pub use system::{is_elevated, kernel_counters};
