//! Columns, filtering, sorting and selection for the process table.

use std::cmp::Ordering;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::core::models::{ProcessInfo, ProcessKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessColumn {
    Name,
    Pid,
    ParentPid,
    Cpu,
    Memory,
    MemoryPercent,
    PrivateBytes,
    Threads,
    Handles,
    User,
    Architecture,
    Session,
    Status,
    Path,
}

impl ProcessColumn {
    pub const ALL: [ProcessColumn; 14] = [
        Self::Name,
        Self::Pid,
        Self::ParentPid,
        Self::Cpu,
        Self::Memory,
        Self::MemoryPercent,
        Self::PrivateBytes,
        Self::Threads,
        Self::Handles,
        Self::User,
        Self::Architecture,
        Self::Session,
        Self::Status,
        Self::Path,
    ];

    pub fn default_visible() -> Vec<ProcessColumn> {
        vec![
            Self::Name,
            Self::Pid,
            Self::ParentPid,
            Self::Cpu,
            Self::Memory,
            Self::MemoryPercent,
            Self::Threads,
            Self::Handles,
            Self::User,
            Self::Architecture,
            Self::Status,
            Self::Path,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Pid => "PID",
            Self::ParentPid => "Parent",
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
            Self::MemoryPercent => "Mem %",
            Self::PrivateBytes => "Private",
            Self::Threads => "Threads",
            Self::Handles => "Handles",
            Self::User => "User",
            Self::Architecture => "Arch",
            Self::Session => "Session",
            Self::Status => "Status",
            Self::Path => "Path",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Self::Name => "Image name",
            Self::Pid => "Process identifier",
            Self::ParentPid => "PID of the process that created this one (it may have exited)",
            Self::Cpu => "CPU utilization across all logical processors since the last sample",
            Self::Memory => "Working set: physical memory currently used by the process",
            Self::MemoryPercent => "Working set as a share of total physical memory",
            Self::PrivateBytes => "Private committed memory that cannot be shared with other processes",
            Self::Threads => "Number of threads",
            Self::Handles => "Number of open kernel object handles",
            Self::User => "Account the process runs as (unavailable for other users without elevation)",
            Self::Architecture => "Image architecture",
            Self::Session => "Terminal Services session ID (0 = services)",
            Self::Status => "Process state",
            Self::Path => "Full path of the executable",
        }
    }

    /// Numeric columns are right-aligned and sort descending first.
    pub fn is_numeric(self) -> bool {
        !matches!(
            self,
            Self::Name | Self::User | Self::Architecture | Self::Status | Self::Path
        )
    }

    pub fn initial_width(self) -> f32 {
        match self {
            Self::Name => 200.0,
            Self::Path => 320.0,
            Self::User => 150.0,
            Self::Pid | Self::ParentPid | Self::Session => 60.0,
            Self::Architecture | Self::Status => 64.0,
            _ => 78.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortSpec {
    pub column: ProcessColumn,
    pub descending: bool,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            column: ProcessColumn::Cpu,
            descending: true,
        }
    }
}

impl SortSpec {
    /// Header click: toggles direction on the same column, otherwise picks a
    /// natural default direction for the new column.
    pub fn clicked(&mut self, column: ProcessColumn) {
        if self.column == column {
            self.descending = !self.descending;
        } else {
            self.column = column;
            self.descending = column.is_numeric();
        }
    }
}

/// Orders `None` after every value regardless of direction.
fn cmp_opt<T: PartialOrd>(a: Option<T>, b: Option<T>, descending: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            let o = a.partial_cmp(&b).unwrap_or(Ordering::Equal);
            if descending { o.reverse() } else { o }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn cmp_str(a: &str, b: &str) -> Ordering {
    // Case-insensitive without allocating.
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

pub fn compare(a: &ProcessInfo, b: &ProcessInfo, sort: SortSpec) -> Ordering {
    use ProcessColumn as C;
    let d = sort.descending;
    let primary = match sort.column {
        C::Name => {
            let o = cmp_str(&a.name, &b.name);
            if d { o.reverse() } else { o }
        }
        C::Pid => cmp_opt(Some(a.key.pid), Some(b.key.pid), d),
        C::ParentPid => cmp_opt(a.parent_pid, b.parent_pid, d),
        C::Cpu => cmp_opt(a.cpu_percent, b.cpu_percent, d),
        C::Memory | C::MemoryPercent => cmp_opt(a.working_set, b.working_set, d),
        C::PrivateBytes => cmp_opt(a.private_bytes, b.private_bytes, d),
        C::Threads => cmp_opt(a.threads, b.threads, d),
        C::Handles => cmp_opt(a.handles, b.handles, d),
        C::Session => cmp_opt(a.session_id, b.session_id, d),
        C::User => cmp_opt(
            a.user.as_deref().map(str::to_lowercase),
            b.user.as_deref().map(str::to_lowercase),
            d,
        ),
        C::Architecture => cmp_opt(
            a.architecture.map(|x| x.label()),
            b.architecture.map(|x| x.label()),
            d,
        ),
        C::Status => cmp_opt(Some(a.status.label()), Some(b.status.label()), d),
        C::Path => cmp_opt(
            a.exe_path.as_deref().map(str::to_lowercase),
            b.exe_path.as_deref().map(str::to_lowercase),
            d,
        ),
    };
    // Stable tie-breaker keeps rows from jumping between refreshes.
    primary.then_with(|| a.key.cmp(&b.key))
}

/// A parsed filter query.
///
/// - empty: matches everything
/// - `pid:123` or a bare number: matches that PID (a bare number also matches names containing it)
/// - anything else: case-insensitive substring of the name or path
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Filter {
    text: String,
    pid: Option<u32>,
    pid_only: bool,
}

impl Filter {
    pub fn parse(query: &str) -> Self {
        let q = query.trim();
        if let Some(rest) = q.strip_prefix("pid:").or_else(|| q.strip_prefix("PID:")) {
            return Self {
                text: String::new(),
                pid: rest.trim().parse().ok(),
                pid_only: true,
            };
        }
        Self {
            text: q.to_lowercase(),
            pid: q.parse().ok(),
            pid_only: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && !self.pid_only
    }

    pub fn matches(&self, p: &ProcessInfo) -> bool {
        if self.pid_only {
            return self.pid == Some(p.key.pid);
        }
        if self.text.is_empty() || self.pid == Some(p.key.pid) {
            return true;
        }
        p.name.to_lowercase().contains(&self.text)
            || p.exe_path
                .as_deref()
                .is_some_and(|path| path.to_lowercase().contains(&self.text))
    }
}

/// Multi-selection with Windows-style click semantics.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    keys: HashSet<ProcessKey>,
    anchor: Option<ProcessKey>,
}

impl Selection {
    pub fn contains(&self, key: &ProcessKey) -> bool {
        self.keys.contains(key)
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The most recently clicked process, used as the "primary" selection.
    pub fn primary(&self) -> Option<ProcessKey> {
        self.anchor.filter(|k| self.keys.contains(k))
    }

    pub fn set_single(&mut self, key: ProcessKey) {
        self.keys.clear();
        self.keys.insert(key);
        self.anchor = Some(key);
    }

    pub fn set_many(&mut self, keys: impl IntoIterator<Item = ProcessKey>) {
        self.keys = keys.into_iter().collect();
        self.anchor = self.keys.iter().next().copied();
    }

    /// Applies a click on `key`. `ordered` is the visible row order, needed for
    /// shift-range selection.
    pub fn click(&mut self, key: ProcessKey, ctrl: bool, shift: bool, ordered: &[ProcessKey]) {
        match (ctrl, shift, self.anchor) {
            (_, true, Some(anchor)) => {
                let a = ordered.iter().position(|k| *k == anchor);
                let b = ordered.iter().position(|k| *k == key);
                if let (Some(a), Some(b)) = (a, b) {
                    if !ctrl {
                        self.keys.clear();
                    }
                    let (lo, hi) = (a.min(b), a.max(b));
                    self.keys.extend(ordered[lo..=hi].iter().copied());
                } else {
                    self.set_single(key);
                }
            }
            (true, _, _) => {
                if !self.keys.remove(&key) {
                    self.keys.insert(key);
                }
                self.anchor = Some(key);
            }
            _ => self.set_single(key),
        }
    }

    /// Drops processes that no longer exist.
    pub fn retain_existing(&mut self, existing: &HashSet<ProcessKey>) {
        self.keys.retain(|k| existing.contains(k));
        if self.anchor.is_some_and(|a| !existing.contains(&a)) {
            self.anchor = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::ProcessKey;

    fn p(pid: u32, name: &str, cpu: Option<f32>) -> ProcessInfo {
        let mut info = ProcessInfo::new(ProcessKey { pid, start_time: 1 }, name);
        info.cpu_percent = cpu;
        info
    }

    #[test]
    fn unavailable_values_sort_last_both_ways() {
        let mut v = [p(1, "a", None), p(2, "b", Some(5.0)), p(3, "c", Some(50.0))];
        let mut sort = SortSpec {
            column: ProcessColumn::Cpu,
            descending: true,
        };
        v.sort_by(|a, b| compare(a, b, sort));
        assert_eq!(v.iter().map(|x| x.key.pid).collect::<Vec<_>>(), vec![3, 2, 1]);
        sort.descending = false;
        v.sort_by(|a, b| compare(a, b, sort));
        assert_eq!(v.iter().map(|x| x.key.pid).collect::<Vec<_>>(), vec![2, 3, 1]);
    }

    #[test]
    fn name_sort_is_case_insensitive() {
        let mut v = [p(1, "beta", None), p(2, "Alpha", None)];
        v.sort_by(|a, b| {
            compare(
                a,
                b,
                SortSpec {
                    column: ProcessColumn::Name,
                    descending: false,
                },
            )
        });
        assert_eq!(&*v[0].name, "Alpha");
    }

    #[test]
    fn header_click_toggles() {
        let mut s = SortSpec::default();
        s.clicked(ProcessColumn::Name);
        assert!(!s.descending);
        s.clicked(ProcessColumn::Name);
        assert!(s.descending);
        s.clicked(ProcessColumn::Memory);
        assert!(s.descending);
    }

    #[test]
    fn filters() {
        let chrome = p(1234, "chrome.exe", None);
        assert!(Filter::parse("").matches(&chrome));
        assert!(Filter::parse("CHROME").matches(&chrome));
        assert!(Filter::parse("1234").matches(&chrome));
        assert!(Filter::parse("pid:1234").matches(&chrome));
        assert!(!Filter::parse("pid:12").matches(&chrome));
        assert!(!Filter::parse("firefox").matches(&chrome));
        assert!(!Filter::parse("pid:").matches(&chrome));
    }

    #[test]
    fn selection_click_semantics() {
        let keys: Vec<_> = (1..=5).map(|pid| ProcessKey { pid, start_time: 0 }).collect();
        let mut s = Selection::default();
        s.click(keys[1], false, false, &keys);
        s.click(keys[3], false, true, &keys);
        assert_eq!(s.len(), 3);
        s.click(keys[2], true, false, &keys);
        assert_eq!(s.len(), 2);
        assert!(!s.contains(&keys[2]));
        s.click(keys[0], false, false, &keys);
        assert_eq!(s.len(), 1);
        assert_eq!(s.primary(), Some(keys[0]));

        let existing: HashSet<_> = keys[1..].iter().copied().collect();
        s.retain_existing(&existing);
        assert!(s.is_empty());
        assert_eq!(s.primary(), None);
    }
}
