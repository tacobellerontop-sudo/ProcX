//! SQLite storage for recorded history.
//!
//! Schema (version 1):
//! - `system_samples`: one row per recording interval.
//! - `processes`: one row per process instance (PID + creation time), with
//!   first/last time seen. PID reuse therefore never merges two processes.
//! - `process_samples`: per-interval values for the busiest processes.
//!
//! Times are Unix milliseconds (UTC).

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::accumulate::Record;

const SCHEMA_VERSION: i32 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS system_samples (
    ts INTEGER PRIMARY KEY,
    cpu_avg REAL NOT NULL,
    cpu_max REAL NOT NULL,
    mem_used INTEGER NOT NULL,
    mem_total INTEGER NOT NULL,
    commit_bytes INTEGER,
    disk_read REAL NOT NULL,
    disk_write REAL NOT NULL,
    net_rx REAL NOT NULL,
    net_tx REAL NOT NULL,
    process_count INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS processes (
    id INTEGER PRIMARY KEY,
    pid INTEGER NOT NULL,
    start_time INTEGER NOT NULL,
    name TEXT NOT NULL,
    exe_path TEXT,
    user TEXT,
    first_seen INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    UNIQUE (pid, start_time)
);
CREATE INDEX IF NOT EXISTS processes_last_seen ON processes (last_seen);
CREATE TABLE IF NOT EXISTS process_samples (
    process_id INTEGER NOT NULL REFERENCES processes (id) ON DELETE CASCADE,
    ts INTEGER NOT NULL,
    cpu REAL,
    working_set INTEGER,
    private_bytes INTEGER,
    PRIMARY KEY (process_id, ts)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS process_samples_ts ON process_samples (ts);
";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SystemPoint {
    pub ts_ms: i64,
    pub cpu_avg: f64,
    pub cpu_max: f64,
    pub mem_used: f64,
    pub mem_total: f64,
    pub commit: Option<f64>,
    pub disk_read: f64,
    pub disk_write: f64,
    pub net_rx: f64,
    pub net_tx: f64,
    pub process_count: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessRow {
    pub id: i64,
    pub pid: u32,
    pub start_time: u64,
    pub name: String,
    pub exe_path: Option<String>,
    pub user: Option<String>,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    /// Aggregates over sampled intervals in the queried range; `None` when
    /// the process was never among the sampled top consumers.
    pub cpu_avg: Option<f64>,
    pub cpu_max: Option<f64>,
    pub working_set_max: Option<u64>,
    pub samples: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessPoint {
    pub ts_ms: i64,
    pub cpu: Option<f64>,
    pub working_set: Option<u64>,
    pub private_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoreStats {
    pub system_rows: u64,
    pub process_rows: u64,
    pub sample_rows: u64,
    pub oldest_ms: Option<i64>,
    pub newest_ms: Option<i64>,
    pub file_bytes: Option<u64>,
}

pub struct Store {
    conn: Connection,
}

type DbResult<T> = Result<T, String>;

fn err(e: rusqlite::Error) -> String {
    format!("database error: {e}")
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn to_u64(v: Option<i64>) -> Option<u64> {
    v.map(|x| u64::try_from(x).unwrap_or(0))
}

impl Store {
    pub fn open(path: &Path) -> DbResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let conn = Connection::open(path).map_err(err)?;
        // WAL keeps readers and the writer from blocking each other and makes
        // each interval's batch a cheap append.
        conn.pragma_update(None, "journal_mode", "WAL").map_err(err)?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> DbResult<Self> {
        Self::init(Connection::open_in_memory().map_err(err)?)
    }

    fn init(conn: Connection) -> DbResult<Self> {
        conn.pragma_update(None, "synchronous", "NORMAL").map_err(err)?;
        conn.pragma_update(None, "foreign_keys", "ON").map_err(err)?;
        conn.busy_timeout(std::time::Duration::from_secs(2))
            .map_err(err)?;
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(err)?;
        if version > SCHEMA_VERSION {
            return Err(format!(
                "the history database was created by a newer ProcX (schema {version}); it was left untouched"
            ));
        }
        conn.execute_batch(SCHEMA).map_err(err)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(err)?;
        Ok(Self { conn })
    }

    /// Writes one interval in a single transaction.
    pub fn insert(&mut self, record: &Record) -> DbResult<()> {
        let tx = self.conn.transaction().map_err(err)?;
        {
            let s = &record.system;
            tx.execute(
                "INSERT OR REPLACE INTO system_samples
                 (ts, cpu_avg, cpu_max, mem_used, mem_total, commit_bytes, disk_read, disk_write, net_rx, net_tx, process_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    s.ts_ms,
                    f64::from(s.cpu_avg),
                    f64::from(s.cpu_max),
                    to_i64(s.mem_used),
                    to_i64(s.mem_total),
                    s.commit.map(to_i64),
                    s.disk_read,
                    s.disk_write,
                    s.net_rx,
                    s.net_tx,
                    s.process_count,
                ],
            )
            .map_err(err)?;
            let mut upsert = tx
                .prepare_cached(
                    "INSERT INTO processes (pid, start_time, name, exe_path, user, first_seen, last_seen)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT (pid, start_time) DO UPDATE SET
                        last_seen = max(last_seen, excluded.last_seen),
                        exe_path = coalesce(exe_path, excluded.exe_path),
                        user = coalesce(user, excluded.user)
                     RETURNING id",
                )
                .map_err(err)?;
            let mut sample = tx
                .prepare_cached(
                    "INSERT OR REPLACE INTO process_samples (process_id, ts, cpu, working_set, private_bytes)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .map_err(err)?;
            for p in &record.processes {
                let id: i64 = upsert
                    .query_row(
                        params![
                            p.key.pid,
                            to_i64(p.key.start_time),
                            p.name,
                            p.exe_path,
                            p.user,
                            p.first_seen_ms,
                            p.last_seen_ms
                        ],
                        |r| r.get(0),
                    )
                    .map_err(err)?;
                if p.sampled {
                    sample
                        .execute(params![
                            id,
                            s.ts_ms,
                            p.cpu_avg.map(f64::from),
                            p.working_set_max.map(to_i64),
                            p.private_max.map(to_i64)
                        ])
                        .map_err(err)?;
                }
            }
        }
        tx.commit().map_err(err)
    }

    /// Deletes everything older than `cutoff_ms`. Returns the rows removed.
    pub fn cleanup(&mut self, cutoff_ms: i64) -> DbResult<usize> {
        let tx = self.conn.transaction().map_err(err)?;
        let mut removed = tx
            .execute("DELETE FROM system_samples WHERE ts < ?1", [cutoff_ms])
            .map_err(err)?;
        removed += tx
            .execute("DELETE FROM process_samples WHERE ts < ?1", [cutoff_ms])
            .map_err(err)?;
        removed += tx
            .execute("DELETE FROM processes WHERE last_seen < ?1", [cutoff_ms])
            .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(removed)
    }

    /// System samples in `[from, to]`. With `max_points`, rows are averaged
    /// into at most that many equal time buckets (peaks keep their maximum).
    pub fn system_series(
        &self,
        from_ms: i64,
        to_ms: i64,
        max_points: Option<usize>,
    ) -> DbResult<Vec<SystemPoint>> {
        let bucket = max_points
            .filter(|n| *n > 0)
            .map_or(1, |n| ((to_ms - from_ms) / n as i64).max(1));
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT max(ts), avg(cpu_avg), max(cpu_max), avg(mem_used), max(mem_total), avg(commit_bytes),
                        avg(disk_read), avg(disk_write), avg(net_rx), avg(net_tx), avg(process_count)
                 FROM system_samples WHERE ts BETWEEN ?1 AND ?2
                 GROUP BY (ts - ?1) / ?3 ORDER BY 1",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(params![from_ms, to_ms, bucket], |r| {
                Ok(SystemPoint {
                    ts_ms: r.get(0)?,
                    cpu_avg: r.get(1)?,
                    cpu_max: r.get(2)?,
                    mem_used: r.get(3)?,
                    mem_total: r.get(4)?,
                    commit: r.get(5)?,
                    disk_read: r.get(6)?,
                    disk_write: r.get(7)?,
                    net_rx: r.get(8)?,
                    net_tx: r.get(9)?,
                    process_count: r.get(10)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Process instances alive at some point in `[from, to]`, with aggregates
    /// of their samples in that range. Ordered by peak CPU, then peak memory.
    pub fn processes(&self, from_ms: i64, to_ms: i64, limit: usize) -> DbResult<Vec<ProcessRow>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT p.id, p.pid, p.start_time, p.name, p.exe_path, p.user, p.first_seen, p.last_seen,
                        avg(s.cpu), max(s.cpu), max(s.working_set), count(s.ts)
                 FROM processes p
                 LEFT JOIN process_samples s ON s.process_id = p.id AND s.ts BETWEEN ?1 AND ?2
                 WHERE p.first_seen <= ?2 AND p.last_seen >= ?1
                 GROUP BY p.id
                 ORDER BY max(s.cpu) IS NULL, max(s.cpu) DESC, max(s.working_set) DESC, p.name
                 LIMIT ?3",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(params![from_ms, to_ms, limit as i64], |r| {
                Ok(ProcessRow {
                    id: r.get(0)?,
                    pid: r.get(1)?,
                    start_time: to_u64(r.get(2)?).unwrap_or(0),
                    name: r.get(3)?,
                    exe_path: r.get(4)?,
                    user: r.get(5)?,
                    first_seen_ms: r.get(6)?,
                    last_seen_ms: r.get(7)?,
                    cpu_avg: r.get(8)?,
                    cpu_max: r.get(9)?,
                    working_set_max: to_u64(r.get(10)?),
                    samples: r.get(11)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn process_series(&self, process_id: i64, from_ms: i64, to_ms: i64) -> DbResult<Vec<ProcessPoint>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT ts, cpu, working_set, private_bytes FROM process_samples
                 WHERE process_id = ?1 AND ts BETWEEN ?2 AND ?3 ORDER BY ts",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(params![process_id, from_ms, to_ms], |r| {
                Ok(ProcessPoint {
                    ts_ms: r.get(0)?,
                    cpu: r.get(1)?,
                    working_set: to_u64(r.get(2)?),
                    private_bytes: to_u64(r.get(3)?),
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn stats(&self) -> DbResult<StoreStats> {
        let count = |sql: &str| -> DbResult<u64> {
            self.conn
                .query_row(sql, [], |r| r.get::<_, i64>(0))
                .map(|v| u64::try_from(v).unwrap_or(0))
                .map_err(err)
        };
        let (oldest, newest) = self
            .conn
            .query_row("SELECT min(ts), max(ts) FROM system_samples", [], |r| {
                Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?))
            })
            .map_err(err)?;
        let file_bytes = self
            .conn
            .query_row(
                "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
                [],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(err)?
            .and_then(|v| u64::try_from(v).ok());
        Ok(StoreStats {
            system_rows: count("SELECT count(*) FROM system_samples")?,
            process_rows: count("SELECT count(*) FROM processes")?,
            sample_rows: count("SELECT count(*) FROM process_samples")?,
            oldest_ms: oldest,
            newest_ms: newest,
            file_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::ProcessKey;
    use crate::history::accumulate::{ProcessRecord, SystemRecord};

    fn record(ts: i64, procs: Vec<ProcessRecord>) -> Record {
        Record {
            system: SystemRecord {
                ts_ms: ts,
                cpu_avg: (ts / 1000) as f32,
                cpu_max: 90.0,
                mem_used: 100,
                mem_total: 1000,
                commit: None,
                disk_read: 1.0,
                disk_write: 2.0,
                net_rx: 3.0,
                net_tx: 4.0,
                process_count: 2,
            },
            processes: procs,
        }
    }

    fn proc(pid: u32, start: u64, ts: i64, cpu: Option<f32>, sampled: bool) -> ProcessRecord {
        ProcessRecord {
            key: ProcessKey {
                pid,
                start_time: start,
            },
            name: format!("p{pid}"),
            exe_path: None,
            user: None,
            first_seen_ms: ts,
            last_seen_ms: ts,
            cpu_avg: cpu,
            working_set_max: Some(50),
            private_max: None,
            sampled,
        }
    }

    #[test]
    fn records_and_queries() {
        let mut db = Store::open_in_memory().unwrap();
        db.insert(&record(
            1_000,
            vec![
                proc(7, 1, 1_000, Some(10.0), true),
                proc(8, 1, 1_000, None, false),
            ],
        ))
        .unwrap();
        db.insert(&record(2_000, vec![proc(7, 1, 2_000, Some(30.0), true)]))
            .unwrap();
        // Same PID, new instance: must be a separate process row.
        db.insert(&record(3_000, vec![proc(7, 2, 3_000, Some(5.0), true)]))
            .unwrap();

        let series = db.system_series(0, 10_000, None).unwrap();
        assert_eq!(series.len(), 3);
        assert_eq!(series[0].cpu_avg, 1.0);

        let rows = db.processes(0, 10_000, 100).unwrap();
        assert_eq!(rows.len(), 3);
        let first = &rows[0];
        assert_eq!((first.pid, first.start_time), (7, 1));
        assert_eq!(first.cpu_avg, Some(20.0));
        assert_eq!(first.cpu_max, Some(30.0));
        assert_eq!((first.first_seen_ms, first.last_seen_ms), (1_000, 2_000));
        let unsampled = rows.iter().find(|r| r.pid == 8).unwrap();
        assert_eq!(unsampled.cpu_avg, None);
        assert_eq!(unsampled.samples, 0);

        let pts = db.process_series(first.id, 0, 10_000).unwrap();
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[1].cpu, Some(30.0));

        // Range filtering uses lifetimes.
        assert_eq!(db.processes(2_500, 10_000, 100).unwrap().len(), 1);
    }

    #[test]
    fn buckets_and_cleanup() {
        let mut db = Store::open_in_memory().unwrap();
        for i in 0..10 {
            db.insert(&record(i * 1_000, vec![proc(1, 1, i * 1_000, Some(1.0), true)]))
                .unwrap();
        }
        let bucketed = db.system_series(0, 9_999, Some(5)).unwrap();
        assert_eq!(bucketed.len(), 5);
        assert_eq!(bucketed[0].cpu_avg, 0.5);
        assert_eq!(bucketed[0].ts_ms, 1_000);

        let removed = db.cleanup(5_000).unwrap();
        assert_eq!(removed, 10);
        let stats = db.stats().unwrap();
        assert_eq!(stats.system_rows, 5);
        assert_eq!(stats.sample_rows, 5);
        assert_eq!(stats.process_rows, 1);
        assert_eq!(stats.oldest_ms, Some(5_000));
        db.cleanup(100_000).unwrap();
        assert_eq!(db.stats().unwrap().process_rows, 0);
    }

    #[test]
    fn reopens_existing_file() {
        let dir = std::env::temp_dir().join(format!("procx-test-{}", std::process::id()));
        let path = dir.join("h.sqlite3");
        {
            let mut db = Store::open(&path).unwrap();
            db.insert(&record(1_000, vec![])).unwrap();
        }
        let db = Store::open(&path).unwrap();
        assert_eq!(db.stats().unwrap().system_rows, 1);
        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }
}
