//! PNG chart rendering for the findings report. Charts visualize the same
//! data already tabulated in the Markdown (load-curve RPC-call rate and
//! latency) — they exist to make the report skimmable, not to introduce new
//! numbers.
//!
//! Uses `plotters`' bitmap backend directly (no font-kit/system-font
//! dependency), so chart generation has no external requirements beyond
//! the `plotters`/`plotters-bitmap` crates already in `Cargo.toml`.

use std::path::{Path, PathBuf};

use plotters::prelude::*;

use super::load_curve::LoadCurvePoint;

const CHART_WIDTH: u32 = 900;
const CHART_HEIGHT: u32 = 380;

// Palette: the validated reference instance from the `dataviz` skill
// (references/palette.md), light-mode slots only — these charts render to
// a static PNG for a Markdown/PDF report, not a themeable page. The three
// categorical slots used here (blue/orange/aqua) are the ones the skill's
// palette validator confirms clear the CVD/normal-vision floors on every
// pairwise comparison, which three simultaneously-plotted line series need.
const SURFACE: RGBColor = RGBColor(0xfc, 0xfc, 0xfb);
const INK_PRIMARY: RGBColor = RGBColor(0x0b, 0x0b, 0x0b);
const INK_SECONDARY: RGBColor = RGBColor(0x52, 0x51, 0x4e);
const GRIDLINE: RGBColor = RGBColor(0xe1, 0xe0, 0xd9);
const AXIS: RGBColor = RGBColor(0xc3, 0xc2, 0xb7);
/// Categorical slot 1 — used alone for the single-series RPC-call-rate
/// chart, and for P50 in the latency chart.
const SERIES_BLUE: RGBColor = RGBColor(0x2a, 0x78, 0xd6);
/// Categorical slot 2 — P95 in the latency chart.
const SERIES_ORANGE: RGBColor = RGBColor(0xeb, 0x68, 0x34);
/// Categorical slot 3 — P99 in the latency chart. Sub-3:1 contrast against
/// `SURFACE` on its own (the palette's documented relief case); the legend
/// plus this report's load-curve table right below each chart are the
/// required relief, so hue is never the only way to identify this series.
const SERIES_AQUA: RGBColor = RGBColor(0x1b, 0xaf, 0x7a);
const LINE_WIDTH: u32 = 2;
const MARKER_RADIUS: i32 = 4; // 8px diameter, the mark spec's marker floor

#[derive(Debug)]
pub struct ChartError(pub String);

impl std::fmt::Display for ChartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "chart rendering error: {}", self.0)
    }
}

impl std::error::Error for ChartError {}

fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Renders an RPC-calls-per-second-over-time line chart (all methods
/// combined — not a confirmed-transaction rate) for one run's load curve to
/// `<dir>/<run_id>_tps.png`. Returns the absolute path written.
pub fn render_tps_chart(
    dir: &Path,
    run_id: &str,
    points: &[LoadCurvePoint],
) -> Result<PathBuf, ChartError> {
    let path = dir.join(format!("{}_tps.png", sanitize_filename(run_id)));
    if points.is_empty() {
        return Err(ChartError("no data points".into()));
    }
    let start = points[0].window_start;
    let series: Vec<(f64, f64)> = points
        .iter()
        .map(|p| {
            (
                (p.window_start - start).num_seconds() as f64,
                p.rpc_calls_per_second,
            )
        })
        .collect();
    let max_x = series.last().map(|(x, _)| *x).unwrap_or(1.0).max(1.0);
    let max_y = series
        .iter()
        .map(|(_, y)| *y)
        .fold(0.0_f64, f64::max)
        .max(1.0)
        * 1.15;

    let backend_path = path.clone();
    let root = BitMapBackend::new(&backend_path, (CHART_WIDTH, CHART_HEIGHT)).into_drawing_area();
    root.fill(&SURFACE).map_err(|e| ChartError(e.to_string()))?;
    let mut chart = ChartBuilder::on(&root)
        .caption(
            format!("{run_id}: RPC calls per second over time"),
            ("sans-serif", 18).into_font().color(&INK_PRIMARY),
        )
        .margin(15)
        .x_label_area_size(35)
        .y_label_area_size(50)
        .build_cartesian_2d(0.0..max_x, 0.0..max_y)
        .map_err(|e| ChartError(e.to_string()))?;
    chart
        .configure_mesh()
        .light_line_style(GRIDLINE)
        .bold_line_style(GRIDLINE)
        .axis_style(AXIS)
        .label_style(("sans-serif", 12).into_font().color(&INK_SECONDARY))
        .x_desc("seconds since run start")
        .y_desc("RPC calls/s")
        .draw()
        .map_err(|e| ChartError(e.to_string()))?;
    chart
        .draw_series(LineSeries::new(
            series.iter().copied(),
            ShapeStyle {
                color: SERIES_BLUE.to_rgba(),
                filled: false,
                stroke_width: LINE_WIDTH,
            },
        ))
        .map_err(|e| ChartError(e.to_string()))?;
    chart
        .draw_series(
            series
                .iter()
                .map(|(x, y)| Circle::new((*x, *y), MARKER_RADIUS, SERIES_BLUE.filled())),
        )
        .map_err(|e| ChartError(e.to_string()))?;
    root.present().map_err(|e| ChartError(e.to_string()))?;
    drop(root);
    Ok(path)
}

/// Renders a P50/P95/P99-latency-over-time line chart for one run's load
/// curve to `<dir>/<run_id>_latency.png`. Returns the absolute path
/// written. Windows with no recorded latency (no successful calls) are
/// skipped from each series rather than plotted as zero.
pub fn render_latency_chart(
    dir: &Path,
    run_id: &str,
    points: &[LoadCurvePoint],
) -> Result<PathBuf, ChartError> {
    let path = dir.join(format!("{}_latency.png", sanitize_filename(run_id)));
    if points.is_empty() {
        return Err(ChartError("no data points".into()));
    }
    let start = points[0].window_start;
    let offset = |p: &LoadCurvePoint| (p.window_start - start).num_seconds() as f64;

    let p50: Vec<(f64, f64)> = points
        .iter()
        .filter_map(|p| p.p50_ms.map(|v| (offset(p), v)))
        .collect();
    let p95: Vec<(f64, f64)> = points
        .iter()
        .filter_map(|p| p.p95_ms.map(|v| (offset(p), v)))
        .collect();
    let p99: Vec<(f64, f64)> = points
        .iter()
        .filter_map(|p| p.p99_ms.map(|v| (offset(p), v)))
        .collect();

    let max_x = points.iter().map(offset).fold(0.0_f64, f64::max).max(1.0);
    let max_y = [&p50, &p95, &p99]
        .iter()
        .flat_map(|s| s.iter().map(|(_, y)| *y))
        .fold(0.0_f64, f64::max)
        .max(1.0)
        * 1.15;

    let backend_path = path.clone();
    let root = BitMapBackend::new(&backend_path, (CHART_WIDTH, CHART_HEIGHT)).into_drawing_area();
    root.fill(&SURFACE).map_err(|e| ChartError(e.to_string()))?;
    let mut chart = ChartBuilder::on(&root)
        .caption(
            format!("{run_id}: latency over time"),
            ("sans-serif", 18).into_font().color(&INK_PRIMARY),
        )
        .margin(15)
        .x_label_area_size(35)
        .y_label_area_size(60)
        .build_cartesian_2d(0.0..max_x, 0.0..max_y)
        .map_err(|e| ChartError(e.to_string()))?;
    chart
        .configure_mesh()
        .light_line_style(GRIDLINE)
        .bold_line_style(GRIDLINE)
        .axis_style(AXIS)
        .label_style(("sans-serif", 12).into_font().color(&INK_SECONDARY))
        .x_desc("seconds since run start")
        .y_desc("latency (ms)")
        .draw()
        .map_err(|e| ChartError(e.to_string()))?;

    // Fixed categorical order (slot 1/2/3), never cycled — P50/P95/P99 are
    // three distinct series a reader tracks via the legend below, not a
    // magnitude gradient of one series.
    for (series, color, label) in [
        (&p50, SERIES_BLUE, "P50"),
        (&p95, SERIES_ORANGE, "P95"),
        (&p99, SERIES_AQUA, "P99"),
    ] {
        let style = ShapeStyle {
            color: color.to_rgba(),
            filled: false,
            stroke_width: LINE_WIDTH,
        };
        chart
            .draw_series(LineSeries::new(series.iter().copied(), style))
            .map_err(|e| ChartError(e.to_string()))?
            .label(label)
            .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], color));
    }
    chart
        .configure_series_labels()
        .background_style(SURFACE.mix(0.9))
        .border_style(AXIS)
        .label_font(("sans-serif", 13).into_font().color(&INK_SECONDARY))
        .draw()
        .map_err(|e| ChartError(e.to_string()))?;
    root.present().map_err(|e| ChartError(e.to_string()))?;
    drop(root);
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::{Backend, RpcCall};
    use crate::report::load_curve::windowed_load_curve;
    use chrono::{TimeZone, Utc};

    fn call_at(secs: i64, latency: Option<u64>) -> RpcCall {
        let base = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        RpcCall {
            call_id: format!("c-{secs}"),
            run_id: "r".into(),
            method: "z_sendmany".into(),
            backend: Backend::Zallet,
            params_hash: None,
            request_at: base + chrono::Duration::seconds(secs),
            response_at: Some(base + chrono::Duration::seconds(secs)),
            latency_ms: latency,
            success: latency.is_some(),
            error_code: None,
            error_message: None,
            phase: crate::data_model::Phase::Unknown,
        }
    }

    #[test]
    fn render_tps_chart_writes_a_png_file() {
        let dir = tempfile::tempdir().unwrap();
        let calls: Vec<RpcCall> = (0..30).map(|i| call_at(i, Some(10))).collect();
        let points = windowed_load_curve(&calls, 10);
        let path = render_tps_chart(dir.path(), "test-run", &points).unwrap();
        assert!(path.exists());
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
    }

    #[test]
    fn render_latency_chart_writes_a_png_file() {
        let dir = tempfile::tempdir().unwrap();
        let calls: Vec<RpcCall> = (0..30).map(|i| call_at(i, Some(10 + i as u64))).collect();
        let points = windowed_load_curve(&calls, 10);
        let path = render_latency_chart(dir.path(), "test-run", &points).unwrap();
        assert!(path.exists());
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
    }

    #[test]
    fn render_tps_chart_empty_points_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(render_tps_chart(dir.path(), "test-run", &[]).is_err());
    }

    #[test]
    fn sanitize_filename_replaces_unsafe_characters() {
        assert_eq!(
            sanitize_filename("2026-08-04T18:04:54Z"),
            "2026-08-04T18_04_54Z"
        );
    }
}
