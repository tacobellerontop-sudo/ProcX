//! CSV export of recorded history (RFC 4180 quoting, UTC ISO-8601 times).

use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime};

use super::store::{ProcessPoint, ProcessRow, SystemPoint};

/// Quotes a field when it contains a delimiter, quote or line break.
pub fn escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

pub fn iso_utc(ts_ms: i64) -> String {
    let t = SystemTime::UNIX_EPOCH + Duration::from_millis(u64::try_from(ts_ms).unwrap_or(0));
    // "YYYY-MM-DD HH:MM:SS UTC" -> "YYYY-MM-DDTHH:MM:SSZ"
    let s = crate::utils::time::format_utc(t);
    format!("{}Z", s.trim_end_matches(" UTC").replacen(' ', "T", 1))
}

fn opt<T: ToString>(v: Option<T>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

pub fn system_csv(out: &mut impl Write, points: &[SystemPoint]) -> std::io::Result<()> {
    writeln!(
        out,
        "time_utc,cpu_avg_percent,cpu_max_percent,memory_used_bytes,memory_total_bytes,commit_bytes,\
         disk_read_bytes_per_sec,disk_write_bytes_per_sec,net_received_bytes_per_sec,net_sent_bytes_per_sec,processes"
    )?;
    for p in points {
        writeln!(
            out,
            "{},{:.2},{:.2},{:.0},{:.0},{},{:.0},{:.0},{:.0},{:.0},{:.0}",
            iso_utc(p.ts_ms),
            p.cpu_avg,
            p.cpu_max,
            p.mem_used,
            p.mem_total,
            opt(p.commit.map(|c| c.round() as u64)),
            p.disk_read,
            p.disk_write,
            p.net_rx,
            p.net_tx,
            p.process_count
        )?;
    }
    Ok(())
}

pub fn process_csv(
    out: &mut impl Write,
    process: &ProcessRow,
    points: &[ProcessPoint],
) -> std::io::Result<()> {
    writeln!(
        out,
        "time_utc,process,pid,cpu_avg_percent,working_set_max_bytes,private_bytes_max"
    )?;
    let name = escape(&process.name);
    for p in points {
        writeln!(
            out,
            "{},{name},{},{},{},{}",
            iso_utc(p.ts_ms),
            process.pid,
            opt(p.cpu.map(|c| format!("{c:.2}"))),
            opt(p.working_set),
            opt(p.private_bytes)
        )?;
    }
    Ok(())
}

/// Writes through a temporary file so a failed export never leaves a
/// truncated CSV behind.
pub fn write_file(
    path: &Path,
    write: impl FnOnce(&mut std::io::BufWriter<std::fs::File>) -> std::io::Result<()>,
) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("csv.tmp");
    let result = (|| {
        let file = std::fs::File::create(&tmp)?;
        let mut w = std::io::BufWriter::new(file);
        write(&mut w)?;
        w.flush()?;
        std::fs::rename(&tmp, path)
    })();
    result.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot write {}: {e}", path.display())
    })
}

/// A file name like `procx-system-20261103-142501.csv`.
pub fn file_name(kind: &str, now_ms: i64) -> String {
    let iso = iso_utc(now_ms);
    let compact: String = iso.chars().filter(char::is_ascii_digit).collect();
    let (date, time) = compact.split_at(8.min(compact.len()));
    let safe: String = kind
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("procx-{safe}-{date}-{time}.csv")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_fields() {
        assert_eq!(escape("plain"), "plain");
        assert_eq!(escape("a,b"), "\"a,b\"");
        assert_eq!(escape("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn formats_times_and_names() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_700_000_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(
            file_name("system", 1_700_000_000_000),
            "procx-system-20231114-221320.csv"
        );
        assert_eq!(file_name("my app.exe", 0), "procx-my_app_exe-19700101-000000.csv");
    }

    #[test]
    fn writes_csv() {
        let row = ProcessRow {
            id: 1,
            pid: 5,
            start_time: 0,
            name: "a,b".into(),
            exe_path: None,
            user: None,
            first_seen_ms: 0,
            last_seen_ms: 0,
            cpu_avg: None,
            cpu_max: None,
            working_set_max: None,
            samples: 1,
        };
        let pts = [ProcessPoint {
            ts_ms: 0,
            cpu: None,
            working_set: Some(10),
            private_bytes: None,
        }];
        let mut buf = Vec::new();
        process_csv(&mut buf, &row, &pts).unwrap();
        let text = String::from_utf8(buf).unwrap();
        let line = text.lines().nth(1).unwrap();
        assert_eq!(line, "1970-01-01T00:00:00Z,\"a,b\",5,,10,");
    }
}
