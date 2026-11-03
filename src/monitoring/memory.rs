//! Physical memory, swap and commit charge.

use sysinfo::System;

use crate::core::models::MemorySnapshot;
use crate::platform::KernelCounters;

pub fn sample(sys: &mut System, kernel: Option<&KernelCounters>) -> MemorySnapshot {
    sys.refresh_memory();
    MemorySnapshot {
        total: sys.total_memory(),
        used: sys.used_memory(),
        available: sys.available_memory(),
        swap_total: sys.total_swap(),
        swap_used: sys.used_swap(),
        commit_total: kernel.map(|k| k.commit_total),
        commit_limit: kernel.map(|k| k.commit_limit).filter(|&l| l > 0),
    }
}
