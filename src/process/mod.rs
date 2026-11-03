//! Process explorer logic: filtering, sorting, tree building, safety rules
//! and the on-demand inspection/action worker.
//!
//! Everything here except `worker` is pure and unit-tested; the worker
//! delegates OS calls to the platform layer.

pub mod details;
pub mod explorer;
pub mod safety;
pub mod tree;
pub mod worker;
