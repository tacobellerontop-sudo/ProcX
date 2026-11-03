//! Windows implementation of the platform layer (via `windows-rs`).

mod disk;
mod files;
mod handle;
mod inspect;
mod processes;
mod services;
mod startup;
mod system;
mod util;

pub use disk::physical_disk_counters;
pub use inspect::{inspect_process, open_file_location, terminate_process};
pub use processes::ProcessEnumerator;
pub use services::{control_service, list_services, set_service_start_type};
pub use startup::{list_startup, set_startup_status};
pub use system::{is_elevated, kernel_counters};
