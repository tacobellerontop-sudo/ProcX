//! Non-Windows fallback: Windows-only information is reported as unavailable.

use super::KernelCounters;

pub fn kernel_counters() -> Option<KernelCounters> {
    None
}

pub fn is_elevated() -> Option<bool> {
    None
}
