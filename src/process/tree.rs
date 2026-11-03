//! Builds the parent/child process tree.
//!
//! Windows records only the *PID* of a process's creator, and PIDs are
//! reused. A parent link is therefore accepted only when the candidate parent
//! was created no later than the child; otherwise the child's real parent has
//! exited and the PID now belongs to an unrelated process, and the child is
//! shown as an orphan at the root.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use super::explorer::{Filter, SortSpec, compare};
use crate::core::models::{ProcessInfo, ProcessKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeRow {
    /// Index into the process slice.
    pub index: usize,
    pub depth: usize,
    pub has_children: bool,
    /// The recorded parent PID no longer refers to the original parent.
    pub orphan: bool,
}

fn is_valid_parent(child: &ProcessInfo, parent: &ProcessInfo) -> bool {
    if parent.key.pid == child.key.pid {
        return false;
    }
    match (parent.started_at, child.started_at) {
        (Some(p), Some(c)) => p <= c,
        _ => parent.key.start_time <= child.key.start_time,
    }
}

/// Index of each process's live parent, if any.
pub fn parent_indices(processes: &[ProcessInfo]) -> Vec<Option<usize>> {
    let by_pid: HashMap<u32, usize> = processes
        .iter()
        .enumerate()
        .map(|(i, p)| (p.key.pid, i))
        .collect();
    processes
        .iter()
        .map(|p| {
            let ppid = p.parent_pid?;
            let &j = by_pid.get(&ppid)?;
            is_valid_parent(p, &processes[j]).then_some(j)
        })
        .collect()
}

/// Flattens the tree into display rows in depth-first order.
///
/// With a non-empty filter, matching processes are shown together with all of
/// their ancestors, and collapsed state is ignored so matches are visible.
pub fn build(
    processes: &[ProcessInfo],
    sort: SortSpec,
    filter: &Filter,
    collapsed: &HashSet<ProcessKey>,
) -> Vec<TreeRow> {
    let parents = parent_indices(processes);
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); processes.len()];
    let mut roots = Vec::new();
    for (i, parent) in parents.iter().enumerate() {
        match parent {
            Some(j) => children[*j].push(i),
            None => roots.push(i),
        }
    }

    let visible: Option<Vec<bool>> = (!filter.is_empty()).then(|| {
        let mut v = vec![false; processes.len()];
        for (i, p) in processes.iter().enumerate() {
            if filter.matches(p) {
                let mut cur = Some(i);
                // Bounded walk guards against malformed (cyclic) parent data.
                for _ in 0..processes.len() {
                    match cur {
                        Some(c) if !v[c] => {
                            v[c] = true;
                            cur = parents[c];
                        }
                        _ => break,
                    }
                }
            }
        }
        v
    });
    let shown = |i: usize| visible.as_ref().is_none_or(|v| v[i]);
    let by = |a: &usize, b: &usize| -> Ordering { compare(&processes[*a], &processes[*b], sort) };

    roots.retain(|&i| shown(i));
    roots.sort_by(by);
    for list in &mut children {
        list.retain(|&i| shown(i));
        list.sort_by(by);
    }

    let mut rows = Vec::with_capacity(processes.len());
    let mut visited = vec![false; processes.len()];
    let mut stack: Vec<(usize, usize)> = roots.iter().rev().map(|&i| (i, 0)).collect();
    while let Some((i, depth)) = stack.pop() {
        if std::mem::replace(&mut visited[i], true) {
            continue;
        }
        let p = &processes[i];
        rows.push(TreeRow {
            index: i,
            depth,
            has_children: !children[i].is_empty(),
            orphan: parents[i].is_none() && p.parent_pid.is_some_and(|pp| pp != 0),
        });
        let expanded = visible.is_some() || !collapsed.contains(&p.key);
        if expanded {
            stack.extend(children[i].iter().rev().map(|&c| (c, depth + 1)));
        }
    }
    // Processes in a parent cycle (malformed data) can never be reached from
    // a root; show them at the root rather than hiding them.
    let reaches_root = |mut i: usize| {
        for _ in 0..=processes.len() {
            match parents[i] {
                None => return true,
                Some(p) => i = p,
            }
        }
        false
    };
    for (i, done) in visited.iter().enumerate() {
        if !done && shown(i) && !reaches_root(i) {
            rows.push(TreeRow {
                index: i,
                depth: 0,
                has_children: false,
                orphan: true,
            });
        }
    }
    rows
}

/// Keys of all live descendants of `root` (not including `root`).
pub fn descendants(processes: &[ProcessInfo], root: ProcessKey) -> Vec<ProcessKey> {
    let parents = parent_indices(processes);
    let Some(root_index) = processes.iter().position(|p| p.key == root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut frontier = vec![root_index];
    let mut seen = vec![false; processes.len()];
    seen[root_index] = true;
    while let Some(cur) = frontier.pop() {
        for (i, parent) in parents.iter().enumerate() {
            if *parent == Some(cur) && !seen[i] {
                seen[i] = true;
                out.push(processes[i].key);
                frontier.push(i);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn p(pid: u32, ppid: Option<u32>, started: u64) -> ProcessInfo {
        let mut info = ProcessInfo::new(
            ProcessKey {
                pid,
                start_time: started,
            },
            format!("p{pid}"),
        );
        info.parent_pid = ppid;
        info.started_at = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(started));
        info
    }

    fn pids(rows: &[TreeRow], procs: &[ProcessInfo]) -> Vec<(u32, usize)> {
        rows.iter().map(|r| (procs[r.index].key.pid, r.depth)).collect()
    }

    const BY_PID: SortSpec = SortSpec {
        column: crate::process::explorer::ProcessColumn::Pid,
        descending: false,
    };

    #[test]
    fn nests_children_under_parents() {
        let procs = vec![
            p(1, None, 0),
            p(10, Some(1), 5),
            p(11, Some(10), 6),
            p(2, Some(1), 3),
        ];
        let rows = build(&procs, BY_PID, &Filter::default(), &HashSet::new());
        assert_eq!(pids(&rows, &procs), vec![(1, 0), (2, 1), (10, 1), (11, 2)]);
        assert!(rows[0].has_children);
    }

    #[test]
    fn reused_parent_pid_makes_orphan() {
        // PID 10 was reused by a process created *after* its "child".
        let procs = vec![p(10, None, 100), p(20, Some(10), 50)];
        let rows = build(&procs, BY_PID, &Filter::default(), &HashSet::new());
        assert_eq!(pids(&rows, &procs), vec![(10, 0), (20, 0)]);
        assert!(rows[1].orphan);
    }

    #[test]
    fn missing_parent_is_orphan_and_pid0_is_not() {
        let procs = vec![p(0, Some(0), 0), p(5, Some(999), 1)];
        let rows = build(&procs, BY_PID, &Filter::default(), &HashSet::new());
        assert!(!rows[0].orphan);
        assert!(rows[1].orphan);
    }

    #[test]
    fn collapse_hides_descendants() {
        let procs = vec![p(1, None, 0), p(2, Some(1), 1), p(3, Some(2), 2)];
        let collapsed: HashSet<_> = [procs[1].key].into();
        let rows = build(&procs, BY_PID, &Filter::default(), &collapsed);
        assert_eq!(pids(&rows, &procs), vec![(1, 0), (2, 1)]);
    }

    #[test]
    fn filter_keeps_ancestors() {
        let procs = vec![p(1, None, 0), p(2, Some(1), 1), p(3, Some(2), 2), p(4, None, 0)];
        let rows = build(&procs, BY_PID, &Filter::parse("pid:3"), &HashSet::new());
        assert_eq!(pids(&rows, &procs), vec![(1, 0), (2, 1), (3, 2)]);
    }

    #[test]
    fn cycles_do_not_hang() {
        let mut a = p(1, Some(2), 0);
        let mut b = p(2, Some(1), 0);
        a.started_at = None;
        b.started_at = None;
        let procs = vec![a, b];
        let rows = build(&procs, BY_PID, &Filter::parse("p1"), &HashSet::new());
        assert_eq!(rows.len(), 2);
        assert!(descendants(&procs, procs[0].key).len() <= 1);
    }

    #[test]
    fn finds_descendants() {
        let procs = vec![p(1, None, 0), p(2, Some(1), 1), p(3, Some(2), 2), p(4, None, 0)];
        let mut d = descendants(&procs, procs[0].key);
        d.sort();
        assert_eq!(d.iter().map(|k| k.pid).collect::<Vec<_>>(), vec![2, 3]);
    }
}
