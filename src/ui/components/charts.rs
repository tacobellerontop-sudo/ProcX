//! Compact time-series charts built on egui_plot.

use egui::Color32;
use egui_plot::{GridMark, HoverPosition, Line, Plot, PlotPoints};

use crate::core::series::TimeSeries;

/// Upper bound of points drawn per series; longer ranges are thinned.
const MAX_POINTS: usize = 600;

pub struct ChartSeries<'a> {
    pub name: &'a str,
    pub data: &'a TimeSeries,
    pub color: Color32,
}

pub struct ChartOptions {
    pub height: f32,
    /// Visible time span in seconds (x axis runs from `-window` to 0).
    pub window_secs: f64,
    /// Fixed upper bound (e.g. 100 for percentages); `None` auto-scales.
    pub y_max: Option<f64>,
    /// Formats y values for hover labels.
    pub format_y: fn(f64) -> String,
}

/// Draws a non-interactive (no pan/zoom) live chart. The newest sample is at x = 0.
pub fn live_chart(ui: &mut egui::Ui, id: &str, now: f64, series: &[ChartSeries<'_>], opts: &ChartOptions) {
    let format_y = opts.format_y;
    let mut plot = Plot::new(id)
        .height(opts.height)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .show_axes([false, true])
        .show_grid([false, true])
        .y_axis_min_width(36.0)
        .y_axis_formatter(move |mark, _| format_y(mark.value))
        .label_formatter(move |hover| match hover {
            HoverPosition::NearDataPoint {
                plot_name, position, ..
            } => Some(format!(
                "{plot_name}\n{:.0}s: {}",
                position.x,
                format_y(position.y)
            )),
            HoverPosition::Elsewhere { .. } => None,
        })
        .include_x(-opts.window_secs)
        .include_x(0.0)
        .include_y(0.0)
        .set_margin_fraction(egui::vec2(0.0, 0.08));
    if let Some(max) = opts.y_max {
        plot = plot
            .include_y(max)
            .auto_bounds([false, false])
            .default_y_bounds(0.0, max);
    } else {
        // A minimum span keeps the axis readable when all values are ~0.
        plot = plot.auto_bounds([false, true]).include_y(1024.0);
    }
    plot = plot.default_x_bounds(-opts.window_secs, 0.0);

    plot.show(ui, |plot_ui| {
        for s in series {
            let points: PlotPoints<'_> = s.data.window_points(now, opts.window_secs, MAX_POINTS).into();
            plot_ui.line(
                Line::new(s.name, points)
                    .color(s.color)
                    .width(1.5)
                    .fill(0.0)
                    .fill_alpha(0.12),
            );
        }
    });
}

/// A series for [`history_chart`]: separate segments are drawn unconnected,
/// so periods without data (ProcX not running) show as gaps.
pub struct HistorySeries {
    pub name: String,
    pub color: Color32,
    pub segments: Vec<Vec<[f64; 2]>>,
    /// Draws a thin line without fill (e.g. a peak overlay).
    pub thin: bool,
}

/// Splits `(unix_ms, value)` points into segments at missing values and at
/// gaps longer than `max_gap_ms`. X values are Unix seconds.
pub fn split_segments(
    points: impl IntoIterator<Item = (i64, Option<f64>)>,
    max_gap_ms: i64,
) -> Vec<Vec<[f64; 2]>> {
    let mut out: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut current: Vec<[f64; 2]> = Vec::new();
    let mut last_ts: Option<i64> = None;
    for (ts, value) in points {
        let gap = last_ts.is_some_and(|l| ts - l > max_gap_ms);
        if (gap || value.is_none()) && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        if let Some(v) = value {
            current.push([ts as f64 / 1000.0, v]);
        }
        last_ts = Some(ts);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Steps (seconds) that make readable clock labels.
const TIME_STEPS: [f64; 14] = [
    60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0, 7200.0, 10800.0, 21600.0, 43200.0, 86400.0, 172800.0,
    604800.0,
];

/// Grid marks on whole local minutes/hours/days, about `target` across the
/// range. `offset_secs` is the local UTC offset.
pub fn time_marks(from: f64, to: f64, target: f64, offset_secs: i32) -> Vec<GridMark> {
    let offset = f64::from(offset_secs);
    let span = (to - from).max(1.0);
    let want = span / target.max(1.0);
    let step = TIME_STEPS
        .iter()
        .copied()
        .find(|s| *s >= want)
        .unwrap_or(TIME_STEPS[TIME_STEPS.len() - 1]);
    let mut v = ((from + offset) / step).ceil() * step - offset;
    let mut marks = Vec::new();
    while v <= to && marks.len() < 100 {
        marks.push(GridMark {
            value: v,
            step_size: step,
        });
        v += step;
    }
    marks
}

pub struct HistoryChartOptions {
    pub height: f32,
    /// Visible range in Unix seconds.
    pub from_secs: f64,
    pub to_secs: f64,
    pub y_max: Option<f64>,
    pub format_y: fn(f64) -> String,
}

/// A chart over absolute time (local when known, else UTC). Charts sharing `link_group` pan and zoom
/// together on the time axis; double-click resets.
pub fn history_chart(
    ui: &mut egui::Ui,
    id: &str,
    link_group: &str,
    series: &[HistorySeries],
    opts: &HistoryChartOptions,
) {
    let format_y = opts.format_y;
    let with_date = opts.to_secs - opts.from_secs > 86_400.0;
    let mut plot = Plot::new(id)
        .height(opts.height)
        .allow_drag([true, false])
        .allow_zoom([true, false])
        .allow_scroll([true, false])
        .allow_boxed_zoom(false)
        .show_grid([true, true])
        .y_axis_min_width(36.0)
        .link_axis(egui::Id::unique(link_group), [true, false])
        .link_cursor(egui::Id::unique(link_group), [true, false])
        .x_grid_spacer(|input| {
            let offset = crate::utils::time::utc_offset().unwrap_or(0);
            time_marks(input.bounds.0, input.bounds.1, 6.0, offset)
        })
        .x_axis_formatter(move |mark, _| crate::utils::time::format_clock(mark.value as i64, with_date))
        .y_axis_formatter(move |mark, _| format_y(mark.value))
        .label_formatter(move |hover| match hover {
            HoverPosition::NearDataPoint {
                plot_name, position, ..
            } => Some(format!(
                "{plot_name}\n{}{}: {}",
                crate::utils::time::format_clock(position.x as i64, true),
                crate::utils::time::zone_suffix(),
                format_y(position.y)
            )),
            HoverPosition::Elsewhere { .. } => None,
        })
        .legend(egui_plot::Legend::default().position(egui_plot::Corner::LeftTop))
        .default_x_bounds(opts.from_secs, opts.to_secs)
        .include_y(0.0)
        .set_margin_fraction(egui::vec2(0.0, 0.08));
    if let Some(max) = opts.y_max {
        plot = plot.default_y_bounds(0.0, max).auto_bounds([false, false]);
    } else {
        plot = plot.auto_bounds([false, true]).include_y(1024.0);
    }
    plot.show(ui, |plot_ui| {
        for s in series {
            for seg in &s.segments {
                let points: PlotPoints<'_> = seg.clone().into();
                let mut line = Line::new(s.name.as_str(), points).color(s.color);
                line = if s.thin {
                    line.width(1.0)
                } else {
                    line.width(1.5).fill(0.0).fill_alpha(0.12)
                };
                plot_ui.line(line);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_gaps_and_missing_values() {
        let pts = vec![
            (0, Some(1.0)),
            (1_000, Some(2.0)),
            (10_000, Some(3.0)),
            (11_000, None),
            (12_000, Some(4.0)),
        ];
        let segs = split_segments(pts, 2_000);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0], vec![[0.0, 1.0], [1.0, 2.0]]);
        assert_eq!(segs[1], vec![[10.0, 3.0]]);
        assert_eq!(segs[2], vec![[12.0, 4.0]]);
        assert!(split_segments(Vec::new(), 1).is_empty());
    }

    #[test]
    fn time_marks_land_on_round_times() {
        let marks = time_marks(1_000.0, 1_000.0 + 3600.0, 6.0, 0);
        assert!(
            marks
                .iter()
                .all(|m| m.step_size == 600.0 && m.value % 600.0 == 0.0)
        );
        assert_eq!(marks.len(), 6);
        assert_eq!(time_marks(0.0, 30.0 * 86_400.0, 6.0, 0)[0].step_size, 604_800.0);
        // UTC+5:30: hourly marks fall on whole local hours (xx:30 UTC).
        let marks = time_marks(0.0, 6.0 * 3600.0, 6.0, 19_800);
        assert!(marks.iter().all(|m| (m.value + 19_800.0) % 3600.0 == 0.0));
    }
}
