//! Recorded history: aggregation, SQLite storage, CSV export and the
//! background thread that ties them together.

pub mod accumulate;
pub mod export;
pub mod store;
pub mod worker;
