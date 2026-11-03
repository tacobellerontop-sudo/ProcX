# ProcX

An advanced process and system monitor for Windows 10/11 (x64), written in Rust with egui.

> **Status: early development.** Overview, Processes, Details, Performance and Settings are functional.
> Other sections are visible in the sidebar but clearly marked as not implemented yet.

## Features so far

- **Overview dashboard** with live CPU, memory, disk and network charts, process/thread/handle
  counts, uptime, commit charge, page file usage, top CPU and memory consumers, health indicators
  and a feed of process starts/exits.
- **Process explorer**: flat or tree view with orphan detection (a parent link is accepted only if
  the parent was created before the child, so reused PIDs are not mistaken for parents), sorting,
  name/path/PID filtering, multi-select, column chooser, resizable columns, context menu (properties,
  open location, copy, go to parent, select children, end process) and keyboard shortcuts
  (`Ctrl+F`, `Ctrl+C`, `Delete`, `Enter`).
- **Safe termination**: confirmation dialog; core Windows processes, processes Windows marks
  critical, and ProcX itself are refused; the process identity (PID + creation time) is re-verified
  immediately before `TerminateProcess` so a recycled PID is never hit.
- **Details**: general info including command line, performance with per-process charts, threads,
  loaded modules, and parent/children, plus the executable's description, company and version. Access-denied parts are reported individually.
- **Performance**: CPU (overall and per logical processor), memory (in use, cache, pools, commit,
  page file), physical disks (active time and throughput via `IOCTL_DISK_PERFORMANCE`, no admin
  needed) plus volumes, and per-interface network throughput, with 1 min to 1 hour ranges. Long
  ranges are thinned for drawing and memory is capped at 3,600 samples per series.
- **Services**: every Win32 service with status, startup type, PID, account, executable,
  dependencies and dependents; search and running/stopped filter. Start, stop and startup-type
  changes go through a confirmation that names running dependents, and the list is re-read from
  Windows afterwards rather than assumed. Changes generally need administrator rights; ProcX
  reports the denial instead of elevating itself.
- **Startup**: entries from the registry Run/RunOnce keys (per-user, all-users and 32-bit) and both
  Startup folders, with publisher from the program's version information. Enable/disable uses the
  same `StartupApproved` flags as Task Manager and Windows Settings, after confirmation; entries
  are never deleted. Scheduled tasks are not listed yet.
- **History**: a background thread records one sample per interval (default 30 s) into SQLite
  (`%APPDATA%\ProcX\history.sqlite3`): system averages and CPU peak, a lifetime row for every
  process instance (keyed by PID + creation time, so reused PIDs never merge), and CPU/memory
  samples for the busiest processes. Writes are one transaction per interval; data older than the
  retention period (default 7 days) is deleted hourly. The History page shows zoomable CPU, memory,
  disk and network charts over 1 hour to 30 days (gaps where ProcX was not running), a searchable
  process table with per-process charts, and CSV export to `%APPDATA%\ProcX\exports` (CSV times are
  UTC ISO-8601; the UI shows local time).
- **Background collector**: all system queries run on a dedicated worker thread and reach the UI
  through a bounded channel, so the UI never blocks on the OS.
- **Settings** persisted as TOML (`%APPDATA%\ProcX\procx.toml`), validated on load with safe
  defaults if the file is missing or invalid.
- Dark theme by default, optional light theme. Keyboard shortcuts: `Ctrl+1`…`Ctrl+8` switch
  sections, `F5` samples immediately, `F1` lists every shortcut.
- Runs without administrator rights. Values the OS cannot provide are shown as `—`, never as `0`.
  When administrator rights are needed (services, all-users startup entries, protected processes),
  **Restart as administrator** in the status bar starts a new instance through the normal UAC
  prompt after a confirmation; ProcX never elevates on its own.
- Times are shown in local time (Windows time zone, including daylight saving); history spanning
  a daylight-saving change is shown with the current offset.
- Panics are written to the log file before the process exits, so release builds (which have no
  console) still leave a trace.

## Building

```powershell
cargo build --release
.\target\release\procx.exe
```

Logs are written to `%APPDATA%\ProcX\procx.log`.

The platform-independent parts also build and test on Linux (`cargo test`), which is useful for
development; Windows-specific values are reported as unavailable there.

## Architecture

```
src/
  main.rs            entry point, logging, window setup
  app/               top-level app state and frame layout; live view model
  core/              models, config, events, errors, health rules, bounded time series
  platform/          OS abstraction; windows/ holds all windows-rs calls
  monitoring/        samplers (cpu, memory, disk, network, processes) and the collector thread
  ui/                theme, navigation, reusable components, one module per page
  history/           aggregation, SQLite store, CSV export and the history thread
  process/           explorer logic, tree, safety checks, worker for inspection and actions
  utils/             formatting, paths, time helpers
```

UI code never calls OS APIs directly; it renders snapshots produced by the collector, and slow or
blocking work (process inspection, services, startup, UAC, SQLite) runs on worker threads that
reply over channels.

## Known limitations

- **No system tray icon.** "Minimize to tray" is saved in Settings but not applied. Recording
  continues while the window is minimized because collection does not depend on the UI.
- **Startup**: scheduled tasks, services and shell extensions that start with Windows are not listed.
- **History** samples CPU and memory only for the busiest processes each interval (15 by CPU plus
  15 by memory); lifetimes are recorded for every process.
- Services and startup changes were tested read-only in CI; changing them needs a real Windows
  session with administrator rights.

## Roadmap

1. ~~Foundation~~ (done)
2. ~~Process explorer: table, tree, details window, safe termination~~ (done)
3. ~~Performance: per-core CPU, memory/page file, per-disk, per-interface charts~~ (done)
4. ~~Windows integration: services and startup managers~~ (done)
5. ~~History: SQLite recording keyed by process instance, charts, CSV export~~ (done)
6. ~~Polish: local time, restart as administrator, shortcut help, icon, crash logging, release
   builds~~ (done; tray icon not implemented, see Known limitations)
