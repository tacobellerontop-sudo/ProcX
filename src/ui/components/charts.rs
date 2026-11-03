//! Compact time-series charts built on egui_plot.

use egui::Color32;
use egui_plot::{HoverPosition, Line, Plot, PlotPoints};

use crate::core::series::TimeSeries;

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
        plot = plot.auto_bounds([false, true]);
    }
    plot = plot.default_x_bounds(-opts.window_secs, 0.0);

    plot.show(ui, |plot_ui| {
        for s in series {
            let points: PlotPoints<'_> = s.data.relative_points(now).into();
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
