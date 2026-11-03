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
  loaded modules, and parent/children. Access-denied parts are reported individually.
- **Performance**: CPU (overall and per logical processor), memory (in use, cache, pools, commit,
  page file), physical disks (active time and throughput via `IOCTL_DISK_PERFORMANCE`, no admin
  needed) plus volumes, and per-interface network throughput, with 1 min to 1 hour ranges. Long
  ranges are thinned for drawing and memory is capped at 3,600 samples per series.
- **Background collector**: all system queries run on a dedicated worker thread and reach the UI
  through a bounded channel, so the UI never blocks on the OS.
- **Settings** persisted as TOML (`%APPDATA%\ProcX\procx.toml`), validated on load with safe
  defaults if the file is missing or invalid.
- Dark theme by default, optional light theme. Keyboard shortcuts: `Ctrl+1`…`Ctrl+8` switch
  sections, `F5` samples immediately.
- Runs without administrator rights. Values the OS cannot provide are shown as `—`, never as `0`.

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
  utils/             formatting, paths, time helpers
```

UI code never calls OS APIs directly; it renders snapshots produced by the collector.

## Roadmap

1. ~~Foundation~~ (done)
2. ~~Process explorer: table, tree, details window, safe termination~~ (done)
3. ~~Performance: per-core CPU, memory/page file, per-disk, per-interface charts~~ (done)
4. Windows integration: services and startup managers
5. History: SQLite recording keyed by process instance, charts, CSV export
6. Polish: tray, more shortcuts, release builds
