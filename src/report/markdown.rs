//! Renders the aggregated run data, RPC matrix, load curves, and candidate
//! findings into a single Markdown document. Candidate findings are
//! rendered as flagged evidence with a mechanical severity tier, not
//! finished prose — see `findings.rs` for why root-cause authorship is a
//! deliberately separate step from this pipeline.

use std::collections::HashMap;
use std::path::Path;

use crate::data_model::{Phase, RpcCall};

use super::charts::{render_latency_chart, render_tps_chart};
use super::findings::{
    flag_candidates, Finding, FindingCategory, Severity, HIGH_DISPARITY_GAP, HIGH_RATE,
    HIGH_TAIL_LATENCY_MULTIPLE, MEDIUM_RATE, MIN_ABSOLUTE_P99_MS, MIN_DISPARITY_GAP,
    MIN_TAIL_LATENCY_SAMPLE, TAIL_LATENCY_MULTIPLE, UNIFORM_SLOWNESS_FLOOR_MS,
};
use super::load_curve::{
    find_degradation_point, peak_tps_point, windowed_load_curve, DEFAULT_WINDOW_SECS,
    DEGRADATION_ERROR_RATE, DEGRADATION_HIGH_LATENCY_MULTIPLE, DEGRADATION_LATENCY_MULTIPLE,
    MIN_WINDOW_CALLS,
};
use super::loader::RunData;
use super::rpc_matrix::{
    build_matrix, build_unlisted, load_parity_annotations, Category, ParityInfo,
};
use super::system_health::{compute_system_health, SystemHealth};

/// Renders a Markdown pipe table with separator-row dash counts
/// proportional to each column's actual max content width.
///
/// Pandoc's LaTeX table writer sizes columns from the separator row's
/// width, not the cell content — a table mixing long Run IDs with short
/// status codes renders every column at equal width otherwise, causing
/// long cells to overflow into their neighbors in the PDF. A single
/// column is capped so it can't starve the others' share of the page.
fn render_table(headers: &[&str], rows: &[Vec<String>], md: &mut String) {
    const MAX_COL_WIDTH: usize = 32;
    const MIN_COL_WIDTH: usize = 3;

    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                *w = (*w).max(cell.chars().count());
            }
        }
    }
    for w in &mut widths {
        *w = (*w).clamp(MIN_COL_WIDTH, MAX_COL_WIDTH);
    }

    md.push('|');
    for h in headers {
        md.push_str(&format!(" {h} |"));
    }
    md.push('\n');
    md.push('|');
    for w in &widths {
        md.push_str(&format!("{}|", "-".repeat(w + 2)));
    }
    md.push('\n');
    for row in rows {
        md.push('|');
        for cell in row {
            md.push_str(&format!(" {cell} |"));
        }
        md.push('\n');
    }
    md.push('\n');
}

/// Executive summary up top: the digest a reader should be able to stop at
/// if they only have a minute — run/scenario counts, overall load results,
/// and a severity-ranked list of the findings that matter most. Everything
/// here is also fully spelled out later in the report; this section exists
/// purely to make the report skimmable, per the project's explicit ask that
/// the report be "comprehensive, easy to read, and already digested."
fn render_executive_summary(runs: &[RunData], findings: &[Finding], md: &mut String) {
    md.push_str("## Executive summary\n\n");

    let scenarios: Vec<&str> = {
        let mut v: Vec<&str> = runs
            .iter()
            .map(|r| r.manifest.scenario_name.as_str())
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    md.push_str(&format!(
        "- **Runs included:** {} (scenario(s): {})\n",
        runs.len(),
        scenarios.join(", ")
    ));

    let attempted: usize = runs.iter().map(|r| r.intents.len()).sum();
    let confirmed: usize = runs
        .iter()
        .flat_map(|r| &r.intents)
        .filter(|i| i.outcome == "confirmed")
        .count();
    let failed: usize = runs
        .iter()
        .flat_map(|r| &r.intents)
        .filter(|i| i.outcome == "failed")
        .count();
    let timed_out: usize = runs
        .iter()
        .flat_map(|r| &r.intents)
        .filter(|i| i.outcome == "timed_out")
        .count();
    let confirmed_pct = if attempted > 0 {
        confirmed as f64 / attempted as f64 * 100.0
    } else {
        0.0
    };
    md.push_str(&format!(
        "- **Intents attempted:** {attempted} — **confirmed {confirmed} ({confirmed_pct:.0}%)**, \
         failed {failed}, timed out {timed_out}\n"
    ));

    let high = findings
        .iter()
        .filter(|f| f.severity == Severity::High)
        .count();
    let medium = findings
        .iter()
        .filter(|f| f.severity == Severity::Medium)
        .count();
    let low = findings
        .iter()
        .filter(|f| f.severity == Severity::Low)
        .count();
    md.push_str(&format!(
        "- **Candidate findings:** **{high} High**, {medium} Medium, {low} Low (see \
         \"Candidate findings\" below; tier definitions in the Appendix)\n\n"
    ));

    md.push_str(
        "**Overall read** (mechanically derived from the counts above — not a \
                  substitute for engineering judgment): ",
    );
    if high == 0 && medium == 0 {
        md.push_str(&format!(
            "no High- or Medium-severity candidates were flagged, and {confirmed_pct:.0}% of \
             attempted intents confirmed across {} run(s).\n\n",
            runs.len()
        ));
    } else if high == 0 {
        md.push_str(&format!(
            "no High-severity candidates were flagged, but {medium} Medium-severity \
             candidate(s) were, alongside a {confirmed_pct:.0}% confirm rate — worth a closer \
             look before treating this load level as clean.\n\n"
        ));
    } else {
        md.push_str(&format!(
            "{high} High-severity candidate(s) were flagged (see below) alongside a \
             {confirmed_pct:.0}% confirm rate — at least one pattern here likely needs \
             engineering follow-up before treating the tested load level as validated.\n\n"
        ));
    }

    let mut high_findings: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.severity == Severity::High)
        .collect();
    high_findings.sort_by(|a, b| {
        a.category
            .to_string()
            .cmp(&b.category.to_string())
            .then(a.summary.cmp(&b.summary))
    });
    if high_findings.is_empty() {
        md.push_str("No High-severity candidates flagged.\n\n");
    } else {
        md.push_str("### High-severity candidates\n\n");
        const MAX_SHOWN: usize = 10;
        for f in high_findings.iter().take(MAX_SHOWN) {
            md.push_str(&format!("- **[{}]** {}\n", f.category, f.summary));
        }
        if high_findings.len() > MAX_SHOWN {
            md.push_str(&format!(
                "- ... and {} more High-severity finding(s) below.\n",
                high_findings.len() - MAX_SHOWN
            ));
        }
        md.push('\n');
    }
}

fn short_sha(s: &str) -> String {
    s.chars().take(8).collect()
}

/// Inserts a zero-width space (U+200B) every `n` characters. Pandoc's LaTeX
/// table writer cannot wrap a single long token with no hyphen/underscore
/// (e.g. `getbestblockheightandhash`) — without a break opportunity it
/// overflows the cell into its neighbor instead of wrapping. The inserted
/// character is invisible in every renderer that matters here (PDF via
/// pandoc, plain Markdown viewers).
fn breakable(s: &str, n: usize) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / n.max(1));
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && i % n == 0 {
            out.push('\u{200B}');
        }
        out.push(ch);
    }
    out
}

fn render_run_metadata(runs: &[RunData], md: &mut String) {
    md.push_str("## Runs included in this report\n\n");
    let headers = [
        "Run ID",
        "Scenario",
        "Target dispatch rate (intents/s)",
        "Zebra",
        "Zaino",
        "Zallet",
        "Simulator commit",
    ];
    let rows: Vec<Vec<String>> = runs
        .iter()
        .map(|run| {
            let m = &run.manifest;
            vec![
                m.run_id.clone(),
                m.scenario_name.clone(),
                m.target_tps.to_string(),
                short_sha(&m.zebra_commit),
                short_sha(&m.zaino_commit),
                short_sha(&m.zallet_commit),
                short_sha(&m.simulator_commit),
            ]
        })
        .collect();
    render_table(&headers, &rows, md);
}

fn render_load_results(runs: &[RunData], md: &mut String) {
    md.push_str("## Load results by run\n\n");
    let headers = ["Run ID", "Attempted", "Confirmed", "Failed", "Timed out"];
    let rows: Vec<Vec<String>> = runs
        .iter()
        .map(|run| {
            let attempted = run.intents.len();
            let confirmed = run
                .intents
                .iter()
                .filter(|i| i.outcome == "confirmed")
                .count();
            let failed = run.intents.iter().filter(|i| i.outcome == "failed").count();
            let timed_out = run
                .intents
                .iter()
                .filter(|i| i.outcome == "timed_out")
                .count();
            vec![
                run.manifest.run_id.clone(),
                attempted.to_string(),
                format!("**{confirmed}**"),
                failed.to_string(),
                timed_out.to_string(),
            ]
        })
        .collect();
    render_table(&headers, &rows, md);
}

/// Confirmed/failed/timed-out breakdown by flow type, shown only for runs
/// whose intents span more than one flow type — a single-flow-type run's
/// breakdown is already fully captured by "Load results by run" above it.
/// Unlike `flow_type_disparity_candidates` in `findings.rs` (which only
/// surfaces a flow type when it trails the overall rate by >20pp), this
/// section is unconditional: a "mixed" scenario's whole point is exercising
/// transparent/shielded flows together, so that comparison shouldn't be
/// hidden behind a disparity threshold.
fn render_flow_type_breakdown(runs: &[RunData], md: &mut String) {
    md.push_str("## Outcomes by flow type\n\n");
    md.push_str(
        "Shown only for runs whose intents span more than one flow type — a single-flow-type \
         run's breakdown is already fully captured by \"Load results by run\" above. Markedly \
         uneven confirm rates across flow types are additionally flagged under \"Flow-type \
         disparity\" in Candidate findings.\n\n",
    );
    let mut any = false;
    for run in runs {
        let mut by_flow: HashMap<String, (u64, u64, u64, u64)> = HashMap::new(); // (confirmed, failed, timed_out, total)
        for intent in &run.intents {
            let flow = format!("{:?}", intent.flow_type);
            let entry = by_flow.entry(flow).or_default();
            entry.3 += 1;
            match intent.outcome.as_str() {
                "confirmed" => entry.0 += 1,
                "failed" => entry.1 += 1,
                "timed_out" => entry.2 += 1,
                _ => {}
            }
        }
        if by_flow.len() < 2 {
            continue;
        }
        any = true;
        md.push_str(&format!("### {}\n\n", run.manifest.run_id));
        let headers = [
            "Flow type",
            "Confirmed",
            "Failed",
            "Timed out",
            "Confirm rate",
        ];
        let mut flows: Vec<_> = by_flow.into_iter().collect();
        flows.sort_by(|a, b| a.0.cmp(&b.0));
        let rows: Vec<Vec<String>> = flows
            .iter()
            .map(|(flow, (confirmed, failed, timed_out, total))| {
                let rate = if *total > 0 {
                    *confirmed as f64 / *total as f64 * 100.0
                } else {
                    0.0
                };
                vec![
                    flow.clone(),
                    confirmed.to_string(),
                    failed.to_string(),
                    timed_out.to_string(),
                    format!("{rate:.0}%"),
                ]
            })
            .collect();
        render_table(&headers, &rows, md);
    }
    if !any {
        md.push_str("No run in this report exercised more than one flow type.\n\n");
    }
}

/// Per-run RPC-call-rate/latency/error-rate curve, all RPC methods combined —
/// the "load-curve results (throughput vs. latency)" scope.md asks for.
/// Scoped to `Load`/`Drain` phase calls only (see
/// `crate::data_model::Phase::is_workload`) — setup-phase activity (bootstrap,
/// warmup mining, funding fan-out) is shown separately, in "Setup phase
/// timing" and "Setup-phase RPC activity" below. Rendered per run, not
/// aggregated, since two runs' time series only make sense side-by-side if
/// they share a load shape (see `load_curve.rs`). When `assets_dir` is
/// provided, an RPC-call-rate chart and a latency chart are rendered as PNGs
/// and embedded after each run's table.
fn render_load_curve(
    runs: &[RunData],
    health: &[SystemHealth],
    assets_dir: Option<&Path>,
    md: &mut String,
) {
    md.push_str("## Load curve by run\n\n");
    md.push_str(&format!(
        "RPC-call rate, latency, and error rate in fixed {DEFAULT_WINDOW_SECS}-second \
         windows across each run's own Load+Drain timeline — all RPC methods combined, \
         setup-phase activity excluded (see \"Setup phase timing\"/\"Setup-phase RPC \
         activity\" for that). Candidate inflection points detected from these curves \
         appear under \"Load degradation\" in Candidate findings; the full RPC compatibility \
         matrix is in the Appendix.\n\n"
    ));
    let fmt_ms = |v: Option<f64>| v.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into());
    let mut any = false;
    for (run, h) in runs.iter().zip(health) {
        let workload_calls: Vec<RpcCall> = run
            .rpc_calls
            .iter()
            .filter(|c| c.phase.is_workload())
            .cloned()
            .collect();
        let points = windowed_load_curve(&workload_calls, DEFAULT_WINDOW_SECS);
        if points.is_empty() {
            continue;
        }
        any = true;
        md.push_str(&format!("### {}\n\n", run.manifest.run_id));

        let confirmed_tps = h
            .confirmed_tx_throughput
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "N/A".into());
        let dispatch_rate = h
            .scheduled_dispatch_rate
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "N/A".into());
        let peak = peak_tps_point(&points);
        let peak_offset = peak
            .map(|p| (p.window_start - points[0].window_start).num_seconds())
            .unwrap_or(0);
        md.push_str(&format!(
            "**Target dispatch rate:** {:.1} intents/s · **Scheduled dispatch rate:** \
             {dispatch_rate} intents/s (run average, from `scheduled_dispatch_rate` — actual \
             load-phase elapsed time, not the configured duration) · **Confirmed tx \
             throughput (TPS):** {confirmed_tps} (from `confirmed_tx_throughput`) · **Peak \
             RPC-call window:** {:.1} calls/s at +{peak_offset}s\n\n",
            run.manifest.target_tps,
            peak.map(|p| p.rpc_calls_per_second).unwrap_or(0.0),
        ));

        match find_degradation_point(&points) {
            Some(d) => md.push_str(&format!(
                "**Candidate degradation point:** +{}s — {:.1} RPC calls/s, P99 {} (baseline \
                 P99 {}), error rate {:.0}% — **[{}]**\n\n",
                d.offset_secs,
                d.rpc_calls_per_second,
                fmt_ms(d.p99_ms),
                fmt_ms(d.baseline_p99_ms),
                d.error_rate * 100.0,
                d.severity,
            )),
            None => md.push_str("No candidate degradation point detected in this run.\n\n"),
        }

        if let Some(dir) = assets_dir {
            if let Ok(path) = render_tps_chart(dir, &run.manifest.run_id, &points) {
                md.push_str(&format!(
                    "![RPC calls per second over time]({})\n\n",
                    path.display()
                ));
            }
            if let Ok(path) = render_latency_chart(dir, &run.manifest.run_id, &points) {
                md.push_str(&format!("![Latency over time]({})\n\n", path.display()));
            }
        }

        let headers = [
            "+s",
            "Calls",
            "Errors",
            "RPC calls/s",
            "P50 ms",
            "P95 ms",
            "P99 ms",
        ];
        let start = points[0].window_start;
        let rows: Vec<Vec<String>> = points
            .iter()
            .map(|p| {
                let offset = (p.window_start - start).num_seconds();
                vec![
                    format!("+{offset}s"),
                    p.calls.to_string(),
                    p.errors.to_string(),
                    format!("{:.1}", p.rpc_calls_per_second),
                    fmt_ms(p.p50_ms),
                    fmt_ms(p.p95_ms),
                    fmt_ms(p.p99_ms),
                ]
            })
            .collect();
        render_table(&headers, &rows, md);
    }
    if !any {
        md.push_str("No RPC calls recorded for the runs in this report.\n\n");
    }
}

/// Figures derived from `metrics.jsonl` rather than `rpc_calls.jsonl` /
/// `intents.jsonl` — mempool depth/saturation, shielded proving time, and
/// per-process CPU/memory. Every other section of this report is silent on
/// these even though the simulator records them on every run (see
/// `system_health.rs`'s module doc); this section is where they surface.
fn render_system_health(runs: &[RunData], health: &[SystemHealth], md: &mut String) {
    md.push_str("## System health by run\n\n");
    md.push_str(
        "Metrics collected during each run beyond the RPC call log: mempool depth and \
         saturation, ZK proving time for shielded withdrawals, and per-process CPU/memory \
         usage. Sourced from `metrics.jsonl` — see docs/architecture/observability.md for what \
         each metric measures and how often it is sampled.\n\n",
    );
    let mut any = false;
    for (run, h) in runs.iter().zip(health) {
        if run.metrics.is_empty() {
            continue;
        }
        any = true;
        md.push_str(&format!("### {}\n\n", run.manifest.run_id));

        match (h.peak_mempool_tx_count, h.peak_mempool_bytes) {
            (None, None) => md.push_str("- **Mempool:** no samples recorded.\n"),
            _ => md.push_str(&format!(
                "- **Mempool:** peak {} tx ({} bytes); {} saturation event(s) observed.\n",
                h.peak_mempool_tx_count
                    .map(|v| format!("{v:.0}"))
                    .unwrap_or_else(|| "—".into()),
                h.peak_mempool_bytes
                    .map(|v| format!("{v:.0}"))
                    .unwrap_or_else(|| "—".into()),
                h.saturation_events,
            )),
        }

        match &h.proving_time {
            Some(p) => md.push_str(&format!(
                "- **Shielded withdrawal proving time:** P50 {:.0}ms, P95 {:.0}ms, P99 {:.0}ms \
                 over {} sample(s).\n",
                p.p50_ms, p.p95_ms, p.p99_ms, p.samples
            )),
            None => md.push_str(
                "- **Shielded withdrawal proving time:** no samples recorded (no shielded \
                 withdrawals observed, or the metric was not emitted).\n",
            ),
        }

        if h.process_peaks.is_empty() {
            md.push_str("- **Resource usage:** no per-process CPU/memory samples recorded.\n\n");
        } else {
            md.push_str("- **Peak resource usage by process:**\n\n");
            let headers = ["Process", "Peak CPU %", "Peak memory (MB)"];
            let rows: Vec<Vec<String>> = h
                .process_peaks
                .iter()
                .map(|p| {
                    vec![
                        p.process.clone(),
                        p.peak_cpu_percent
                            .map(|v| format!("{v:.1}"))
                            .unwrap_or_else(|| "—".into()),
                        p.peak_memory_mb
                            .map(|v| format!("{v:.0}"))
                            .unwrap_or_else(|| "—".into()),
                    ]
                })
                .collect();
            render_table(&headers, &rows, md);
        }
    }
    if !any {
        md.push_str("No metrics recorded for the runs in this report.\n\n");
    }
}

fn render_rpc_matrix(runs: &[RunData], md: &mut String) {
    md.push_str("## RPC compatibility matrix\n\n");
    md.push_str(
        "Derived mechanically from the `rpc_calls.jsonl` of every run listed above, scoped to \
         `Load`/`Drain`-phase calls only — the measured workload. Setup-phase activity \
         (bootstrap, warmup mining, funding fan-out) is shown separately in \"Setup-phase RPC \
         activity\" below, not folded into this matrix. \
         `Not tested` means no observed call in this scope, not \"known to fail\". \
         `Parity`/`Notes` are pulled in from the hand-maintained docs/rpc/rpc-coverage-matrix.md \
         — they reflect that doc's most recent update, not this report's own runs; `TBD` means \
         \"not yet independently verified,\" not a claim of correctness.\n\n",
    );
    let matrix = build_matrix(runs, Phase::is_workload);
    let parity = load_parity_annotations();
    let empty_parity = ParityInfo::default();
    let headers = [
        "Method",
        "Backend",
        "Status",
        "Calls",
        "Successes",
        "P50 ms",
        "P95 ms",
        "P99 ms",
        "Error codes",
        "Parity",
        "Notes",
    ];
    for category in [Category::Stress, Category::RegtestControl, Category::Smoke] {
        let cat_rows: Vec<_> = matrix.iter().filter(|r| r.category == category).collect();
        if cat_rows.is_empty() {
            continue;
        }
        md.push_str(&format!("### {category}\n\n"));
        let fmt_ms = |v: Option<f64>| v.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into());
        let rows: Vec<Vec<String>> = cat_rows
            .iter()
            .map(|row| {
                let codes = if row.error_codes.is_empty() {
                    "—".to_string()
                } else {
                    format!("{:?}", row.error_codes)
                };
                let info = parity.get(row.method).unwrap_or(&empty_parity);
                vec![
                    breakable(row.method, 8),
                    row.backend_label.to_string(),
                    row.status.to_string(),
                    row.calls.to_string(),
                    row.successes.to_string(),
                    fmt_ms(row.p50_ms),
                    fmt_ms(row.p95_ms),
                    fmt_ms(row.p99_ms),
                    codes,
                    if info.parity.is_empty() {
                        "—".to_string()
                    } else {
                        info.parity.clone()
                    },
                    if info.notes.is_empty() {
                        "—".to_string()
                    } else {
                        info.notes.clone()
                    },
                ]
            })
            .collect();
        render_table(&headers, &rows, md);
    }

    render_unlisted_rpc_calls(runs, md);
}

/// RPC calls observed during these runs whose method is not part of the
/// tracked roster — see [`build_unlisted`]'s doc comment for why this
/// exists as a separate section instead of silently dropping them, as
/// `build_matrix` does.
fn render_unlisted_rpc_calls(runs: &[RunData], md: &mut String) {
    let rows = build_unlisted(runs, Phase::is_workload);
    if rows.is_empty() {
        return;
    }
    md.push_str("### Observed outside the tracked roster\n\n");
    md.push_str(&format!(
        "{} method(s) were called during these runs but are not part of the \
         {}-method roster tracked above (see docs/rpc/rpc-coverage-matrix.md) — informational \
         only, not a coverage gap or a defect.\n\n",
        rows.len(),
        super::rpc_matrix::IN_SCOPE_METHODS.len(),
    ));
    let headers = [
        "Method",
        "Backend(s)",
        "Status",
        "Calls",
        "Successes",
        "P50 ms",
        "P95 ms",
        "P99 ms",
        "Error codes",
    ];
    let fmt_ms = |v: Option<f64>| v.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into());
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            let codes = if row.error_codes.is_empty() {
                "—".to_string()
            } else {
                format!("{:?}", row.error_codes)
            };
            vec![
                breakable(&row.method, 8),
                row.observed_backends.join(", "),
                row.status.to_string(),
                row.calls.to_string(),
                row.successes.to_string(),
                fmt_ms(row.p50_ms),
                fmt_ms(row.p95_ms),
                fmt_ms(row.p99_ms),
                codes,
            ]
        })
        .collect();
    render_table(&headers, &table_rows, md);
}

/// Setup-phase (`Bootstrap`/`Readiness`/`Warmup`/`Funding`) RPC activity —
/// built the same way as the headline RPC compatibility matrix, but scoped
/// to the opposite phase set (see [`Phase::is_setup`]) and deliberately never
/// fed into `findings.rs`'s rate-based severity scoring: this is diagnostic
/// context on setup behavior (including funding's own anchor-confirmation
/// retries), not a workload finding.
fn render_setup_phase_rpc_activity(runs: &[RunData], md: &mut String) {
    md.push_str("## Setup-phase RPC activity\n\n");
    md.push_str(
        "Informational only — never scored as a candidate finding. Covers every RPC call \
         issued before the measured workload began: stack bootstrap, hot-wallet readiness \
         polling, warmup mining, and the funding fan-out (including its own anchor-confirmation \
         retries).\n\n",
    );
    let matrix = build_matrix(runs, Phase::is_setup);
    let exercised: Vec<_> = matrix.iter().filter(|r| r.calls > 0).collect();
    let unlisted = build_unlisted(runs, Phase::is_setup);
    if exercised.is_empty() && unlisted.is_empty() {
        md.push_str("No setup-phase RPC activity recorded for the runs in this report.\n\n");
        return;
    }
    let fmt_ms = |v: Option<f64>| v.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into());
    if !exercised.is_empty() {
        let headers = [
            "Method",
            "Backend",
            "Calls",
            "Successes",
            "P50 ms",
            "P95 ms",
            "P99 ms",
            "Error codes",
        ];
        let rows: Vec<Vec<String>> = exercised
            .iter()
            .map(|row| {
                let codes = if row.error_codes.is_empty() {
                    "—".to_string()
                } else {
                    format!("{:?}", row.error_codes)
                };
                vec![
                    breakable(row.method, 8),
                    row.backend_label.to_string(),
                    row.calls.to_string(),
                    row.successes.to_string(),
                    fmt_ms(row.p50_ms),
                    fmt_ms(row.p95_ms),
                    fmt_ms(row.p99_ms),
                    codes,
                ]
            })
            .collect();
        render_table(&headers, &rows, md);
    }
    if !unlisted.is_empty() {
        // Symmetric with the headline matrix's own "Observed outside the
        // tracked roster" — an off-roster method exercised only during setup
        // must not become invisible just because it isn't Load/Drain scoped.
        md.push_str("Also observed outside the tracked roster during setup:\n\n");
        let headers = [
            "Method",
            "Backend(s)",
            "Calls",
            "Successes",
            "P50 ms",
            "P95 ms",
            "P99 ms",
            "Error codes",
        ];
        let rows: Vec<Vec<String>> = unlisted
            .iter()
            .map(|row| {
                let codes = if row.error_codes.is_empty() {
                    "—".to_string()
                } else {
                    format!("{:?}", row.error_codes)
                };
                vec![
                    breakable(&row.method, 8),
                    row.observed_backends.join(", "),
                    row.calls.to_string(),
                    row.successes.to_string(),
                    fmt_ms(row.p50_ms),
                    fmt_ms(row.p95_ms),
                    fmt_ms(row.p99_ms),
                    codes,
                ]
            })
            .collect();
        render_table(&headers, &rows, md);
    }
}

/// One line per run noting any `Phase::Unknown` RPC calls in its evidence —
/// rows from a run directory predating phase tagging. Deliberately not
/// silently dropped nor silently folded into `Load` (which would mislabel
/// setup-phase evidence as workload evidence on the very next report run
/// against an old run directory).
fn render_unknown_phase_advisory(runs: &[RunData], md: &mut String) {
    let affected: Vec<(&str, usize)> = runs
        .iter()
        .map(|r| {
            (
                r.manifest.run_id.as_str(),
                r.rpc_calls
                    .iter()
                    .filter(|c| c.phase == Phase::Unknown)
                    .count(),
            )
        })
        .filter(|(_, count)| *count > 0)
        .collect();
    if affected.is_empty() {
        return;
    }
    for (run_id, count) in affected {
        md.push_str(&format!(
            "> **Note:** {run_id}: {count} RPC call(s) have unknown phase — this run predates \
             phase instrumentation and is excluded from every phase-scoped view above (both the \
             workload matrix/load curve and the setup-phase appendix); regenerate a newer run \
             for phase-scoped analysis.\n\n"
        ));
    }
}

/// "Setup phase timing" — phase name, start timestamp, and duration (the
/// delta to the next boundary, or to `run_completed_at` for the last phase)
/// from `manifest.phase_boundaries`. Kept separate from "Load curve by run"
/// so setup timing remains visible without it being read as part of the
/// measured workload.
fn render_setup_phase_timing(runs: &[RunData], md: &mut String) {
    md.push_str("## Setup phase timing\n\n");
    md.push_str(
        "Wall-clock start time and duration of each lifecycle phase, from the run manifest. \
         `Load`/`Drain` are included here for completeness but are the measured workload — see \
         \"Load curve by run\" for their own detail.\n\n",
    );
    let mut any = false;
    for run in runs {
        let boundaries = &run.manifest.phase_boundaries;
        if boundaries.is_empty() {
            continue;
        }
        any = true;
        md.push_str(&format!("### {}\n\n", run.manifest.run_id));
        let headers = ["Phase", "Started at", "Duration"];
        let mut rows: Vec<Vec<String>> = boundaries
            .iter()
            .enumerate()
            .map(|(i, b)| {
                // The last boundary (always Drain, once any phase was
                // reached at all) ends when `load_phase()` itself returned
                // — NOT `run_completed_at`, which additionally includes Z3
                // stack teardown time. Falls back to `run_completed_at` only
                // for a manifest that predates this field, or a run whose
                // load phase never completed (setup failed before Drain).
                let end = boundaries
                    .get(i + 1)
                    .map(|next| next.started_at)
                    .or(run.manifest.load_and_drain_completed_at)
                    .or(run.manifest.run_completed_at);
                let duration = end
                    .map(|e| format!("{}s", (e - b.started_at).num_seconds().max(0)))
                    .unwrap_or_else(|| "—".to_string());
                vec![
                    format!("{:?}", b.phase),
                    b.started_at.format("%H:%M:%S%.3f").to_string(),
                    duration,
                ]
            })
            .collect();
        // The residual gap between the Drain phase's own end and
        // `run_completed_at` is Z3 stack teardown — shown explicitly rather
        // than silently absorbed into Drain's duration, so cross-checking
        // Drain's duration against `confirmed_tx_throughput`'s own
        // elapsed-time window (which also stops at `load_and_drain_completed_at`)
        // reconciles.
        if let (Some(completed), Some(load_and_drain_done)) = (
            run.manifest.run_completed_at,
            run.manifest.load_and_drain_completed_at,
        ) {
            let teardown_secs = (completed - load_and_drain_done).num_seconds();
            if teardown_secs > 0 {
                rows.push(vec![
                    "Teardown".to_string(),
                    load_and_drain_done.format("%H:%M:%S%.3f").to_string(),
                    format!("{teardown_secs}s"),
                ]);
            }
        }
        render_table(&headers, &rows, md);
    }
    if !any {
        md.push_str(
            "No phase-boundary data recorded for the runs in this report (these runs predate \
             phase instrumentation).\n\n",
        );
    }
}

fn render_findings(findings: &[Finding], md: &mut String) {
    md.push_str("## Candidate findings\n\n");
    md.push_str(
        "**These are flagged candidates, not finished findings.** Each is a mechanically \
         detected statistical outlier or observed failure, stated with only what the data \
         shows. `Severity` is a simple rate/ratio-based tier meant to triage this list, not a \
         substitute for reading the evidence (exact thresholds are in the Appendix); root \
         cause and remediation recommendations still require judgment and are not assigned \
         here. Within each category, findings are listed High severity first.\n\n",
    );
    if findings.is_empty() {
        md.push_str("No candidates flagged for the runs included in this report.\n\n");
        return;
    }
    for category in [
        FindingCategory::RpcFailure,
        FindingCategory::Timeout,
        FindingCategory::LatencyOutlier,
        FindingCategory::FlowTypeDisparity,
        FindingCategory::LoadDegradation,
        FindingCategory::DataCompleteness,
    ] {
        let mut items: Vec<&Finding> = findings.iter().filter(|f| f.category == category).collect();
        if items.is_empty() {
            continue;
        }
        items.sort_by_key(|f| f.severity);
        md.push_str(&format!("### {category}\n\n"));
        for item in items {
            md.push_str(&format!("- **[{}]** {}\n", item.severity, item.summary));
            for ev in &item.evidence {
                md.push_str(&format!("  - {ev}\n"));
            }
        }
        md.push('\n');
    }
}

fn render_limitations(runs: &[RunData], md: &mut String) {
    md.push_str("## Limitations and threats to validity\n\n");
    md.push_str(&format!(
        "- This report covers {} run(s). Scenario coverage may be incomplete — \
         see the scenario library's design doc \
         (`docs/scenarios/scenario-design.md`) for the full intended battery.\n",
        runs.len()
    ));
    md.push_str(
        "- The RPC compatibility matrix's `Not tested` methods have not been exercised by \
         any run in this report; that is a coverage gap, not evidence of a defect.\n",
    );
    md.push_str(
        "- Severity tiers are simple rate/ratio thresholds, not a human severity assessment — \
         see the Appendix for exact definitions.\n",
    );
    let warning_runs: Vec<_> = runs
        .iter()
        .filter(|r| !r.parse_warnings.is_empty())
        .collect();
    if !warning_runs.is_empty() {
        md.push_str(&format!(
            "- {} run(s) had malformed lines skipped during loading — see \
             \"Data completeness\" findings above for exactly which lines and which runs.\n",
            warning_runs.len()
        ));
    }
    md.push('\n');
}

/// Documents the exact mechanical rule behind each severity tier, so
/// "High"/"Medium"/"Low" in the report above can be checked against a
/// precise definition rather than taken on faith.
fn render_severity_appendix(md: &mut String) {
    md.push_str("## Appendix: severity tier definitions\n\n");
    md.push_str(
        "Severity is assigned mechanically from simple rate/ratio thresholds — a triage \
         aid for this list, not a human severity assessment. Exact rule per category:\n\n",
    );
    md.push_str(&format!(
        "- **RPC failure** / **Timeout** — by failure/timeout rate: \
         `>= {:.0}%` = **High**, `>= {:.0}%` = **Medium**, otherwise **Low**.\n",
        HIGH_RATE * 100.0,
        MEDIUM_RATE * 100.0,
    ));
    md.push_str(&format!(
        "- **Latency outlier** — flagged by either of two independent checks, over at least \
         `{MIN_TAIL_LATENCY_SAMPLE}` calls (below that, latency is treated as noise), and \
         reported once per method even if both checks would otherwise fire: \
         (1) *tail*: P99 reaches `{TAIL_LATENCY_MULTIPLE:.0}x` its own P50 *and* P99 is at \
         least `{MIN_ABSOLUTE_P99_MS:.0}ms` — **High** at `>= {HIGH_TAIL_LATENCY_MULTIPLE:.0}x` \
         P50, otherwise **Medium**; (2) *uniform*: P50 alone reaches \
         `{UNIFORM_SLOWNESS_FLOOR_MS:.0}ms` regardless of ratio — always **High**, and, unlike \
         check (1), not exempt for regtest-control methods (`generate` and friends), since a \
         pathologically slow control-plane call is itself a signal worth surfacing.\n",
    ));
    md.push_str(&format!(
        "- **Flow-type disparity** — a flow type is flagged at all only when its confirm \
         rate trails the overall rate by more than `{:.0}` percentage points. Once flagged: \
         `>= {:.0}pp` gap = **High**, otherwise **Medium**.\n",
        MIN_DISPARITY_GAP * 100.0,
        HIGH_DISPARITY_GAP * 100.0,
    ));
    md.push_str(&format!(
        "- **Load degradation** — a time window (of at least `{MIN_WINDOW_CALLS}` calls, to \
         avoid noise) is flagged when its P99 reaches `{:.0}x` the run's own baseline window, \
         *or* its error rate reaches `{:.0}%`. Once flagged: **High** if the latency ratio \
         reaches `{:.0}x` or the error rate reaches `{:.0}%`, otherwise **Medium**. At most \
         one candidate is flagged per run.\n",
        DEGRADATION_LATENCY_MULTIPLE,
        DEGRADATION_ERROR_RATE * 100.0,
        DEGRADATION_HIGH_LATENCY_MULTIPLE,
        DEGRADATION_ERROR_RATE * 200.0,
    ));
    md.push_str(
        "- **Data completeness** — always **Low**. Flags incomplete evidence (malformed \
         input lines), not a claim about the system under test.\n\n",
    );
}

fn render_report_impl(runs: &[RunData], assets_dir: Option<&Path>) -> String {
    let findings = flag_candidates(runs);
    let health: Vec<SystemHealth> = runs.iter().map(compute_system_health).collect();
    let mut md = String::new();
    md.push_str("# Z3 Exchange Simulator — Findings Report\n\n");
    render_executive_summary(runs, &findings, &mut md);
    render_run_metadata(runs, &mut md);
    render_load_results(runs, &mut md);
    render_flow_type_breakdown(runs, &mut md);
    render_load_curve(runs, &health, assets_dir, &mut md);
    render_system_health(runs, &health, &mut md);
    render_setup_phase_timing(runs, &mut md);
    render_rpc_matrix(runs, &mut md);
    render_setup_phase_rpc_activity(runs, &mut md);
    render_unknown_phase_advisory(runs, &mut md);
    render_findings(&findings, &mut md);
    render_limitations(runs, &mut md);
    render_severity_appendix(&mut md);
    md
}

/// Renders the full Markdown report for the given runs, without charts.
pub fn render_report(runs: &[RunData]) -> String {
    render_report_impl(runs, None)
}

/// Renders the full Markdown report, additionally generating an RPC-call-rate
/// chart and a latency chart per run into `assets_dir` (created if missing) and
/// embedding them via absolute file paths — simplest way to keep the
/// images resolvable regardless of the working directory a PDF converter
/// is run from.
pub fn render_report_with_assets(runs: &[RunData], assets_dir: &Path) -> String {
    let _ = std::fs::create_dir_all(assets_dir);
    render_report_impl(runs, Some(assets_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::{Backend, FlowType, IntentRecord, RpcCall};
    use crate::metrics::{RunManifest, RunTimeouts};
    use chrono::Utc;

    fn sample_run() -> RunData {
        RunData {
            run_dir: "/tmp/r1".into(),
            manifest: RunManifest {
                run_id: "20260803T084825Z-smoke".into(),
                run_started_at: Utc::now(),
                run_completed_at: Some(Utc::now()),
                simulator_commit: "5d6340a2bbcf938a80fe2db661dd70f93bd6c9ea".into(),
                zebra_commit: "bb41d69013edbfa8594bb097fa751f47eeb31445".into(),
                zaino_commit: "17963672d0c2cad97dd12bd38bbf1b6fd232c8c5".into(),
                zallet_commit: "bd7f020eb9e1de6f79da947e8102281832b05f83".into(),
                scenario_name: "smoke".into(),
                scenario_config_hash: "sha256:x".into(),
                target_tps: 1.0,
                timeouts: RunTimeouts::default(),
                phase_boundaries: Vec::new(),
                load_and_drain_completed_at: None,
            },
            rpc_calls: vec![
                RpcCall {
                    call_id: "c1".into(),
                    run_id: "r".into(),
                    method: "getblockcount".into(),
                    backend: Backend::Zebra,
                    params_hash: None,
                    request_at: Utc::now(),
                    response_at: Some(Utc::now()),
                    latency_ms: Some(5),
                    success: true,
                    error_code: None,
                    error_message: None,
                    phase: crate::data_model::Phase::Load,
                },
                RpcCall {
                    call_id: "c2".into(),
                    run_id: "r".into(),
                    method: "z_listunspent".into(),
                    backend: Backend::Zallet,
                    params_hash: None,
                    request_at: Utc::now(),
                    response_at: Some(Utc::now()),
                    latency_ms: None,
                    success: false,
                    error_code: Some(-20),
                    error_message: Some("WalletDb::get_memo failed".into()),
                    phase: crate::data_model::Phase::Load,
                },
            ],
            intents: vec![
                IntentRecord {
                    run_id: "r".into(),
                    intent_id: "i1".into(),
                    flow_type: FlowType::TToT,
                    outcome: "confirmed".into(),
                    error: None,
                    timeout_context: None,
                    recorded_at: Utc::now(),
                },
                IntentRecord {
                    run_id: "r".into(),
                    intent_id: "i2".into(),
                    flow_type: FlowType::TToT,
                    outcome: "failed".into(),
                    error: Some("insufficient balance".into()),
                    timeout_context: None,
                    recorded_at: Utc::now(),
                },
            ],
            metrics: Vec::new(),
            parse_warnings: Vec::new(),
        }
    }

    #[test]
    fn render_report_includes_run_id_and_commits() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("20260803T084825Z-smoke"));
        assert!(md.contains("bb41d690"));
    }

    #[test]
    fn render_report_includes_executive_summary() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("## Executive summary"));
        assert!(md.contains("Runs included:** 1"));
        assert!(md.contains("Intents attempted:** 2"));
    }

    #[test]
    fn render_report_includes_load_results() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("## Load results by run"));
        assert!(md.contains("20260803T084825Z-smoke"));
        assert!(md.contains("**1**")); // confirmed count, bolded
    }

    #[test]
    fn render_report_includes_load_curve_section() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("## Load curve by run"));
        assert!(md.contains("20260803T084825Z-smoke"));
        // Rendered from the `confirmed_tx_throughput` metric (see
        // `render_load_curve`) — the only rate labeled "TPS" in this report.
        assert!(md.contains("Confirmed tx throughput (TPS)"));
    }

    #[test]
    fn render_report_includes_rpc_matrix_with_status() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("## RPC compatibility matrix"));
        assert!(md.contains("z_listun")); // method name may carry break hints past this point
        assert!(md.contains("Failed"));
    }

    #[test]
    fn render_report_includes_candidate_findings_with_severity_tag() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("## Candidate findings"));
        assert!(md.contains("not finished findings"));
        assert!(md.contains("z_listunspent"));
        assert!(
            md.contains("**[High]**") || md.contains("**[Medium]**") || md.contains("**[Low]**")
        );
    }

    #[test]
    fn render_report_no_findings_states_so_explicitly() {
        let mut run = sample_run();
        run.rpc_calls.retain(|c| c.success);
        run.intents = vec![IntentRecord {
            run_id: "r".into(),
            intent_id: "i1".into(),
            flow_type: FlowType::TToT,
            outcome: "confirmed".into(),
            error: None,
            timeout_context: None,
            recorded_at: Utc::now(),
        }];
        let md = render_report(&[run]);
        assert!(md.contains("No candidates flagged"));
        assert!(md.contains("No High-severity candidates flagged"));
    }

    #[test]
    fn render_report_includes_limitations_section() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("## Limitations and threats to validity"));
        assert!(md.contains("1 run(s)"));
    }

    #[test]
    fn render_report_includes_severity_appendix() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("## Appendix: severity tier definitions"));
        assert!(md.contains("RPC failure"));
        assert!(md.contains("Load degradation"));
    }

    #[test]
    fn render_report_empty_runs_does_not_panic() {
        let md = render_report(&[]);
        assert!(md.contains("# Z3 Exchange Simulator"));
        assert!(md.contains("No RPC calls recorded"));
    }

    #[test]
    fn unknown_phase_calls_produce_advisory_not_silent_drop() {
        let mut run = sample_run();
        run.rpc_calls.push(RpcCall {
            call_id: "c-unknown".into(),
            run_id: "r".into(),
            method: "getblockcount".into(),
            backend: crate::data_model::Backend::Zebra,
            params_hash: None,
            request_at: chrono::Utc::now(),
            response_at: Some(chrono::Utc::now()),
            latency_ms: Some(5),
            success: true,
            error_code: None,
            error_message: None,
            phase: Phase::Unknown,
        });
        let md = render_report(&[run]);
        assert!(
            md.contains("unknown phase"),
            "expected an advisory line naming unknown-phase calls; got:\n{md}"
        );
        assert!(
            md.contains("predates phase instrumentation"),
            "advisory must explain why, not just flag the count; got:\n{md}"
        );
    }

    #[test]
    fn render_report_without_unknown_phase_calls_omits_the_advisory() {
        let md = render_report(&[sample_run()]);
        assert!(!md.contains("predates phase instrumentation"));
    }

    #[test]
    fn render_report_includes_setup_phase_sections() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("## Setup phase timing"));
        assert!(md.contains("## Setup-phase RPC activity"));
    }

    #[test]
    fn setup_phase_timing_drain_duration_excludes_teardown_and_shows_it_separately() {
        // Regression guard: the Drain row's duration must reconcile with
        // `confirmed_tx_throughput`'s own elapsed-time window — both stop at
        // `load_and_drain_completed_at`, not `run_completed_at` (which also
        // includes Z3 stack teardown time). The gap between the two must
        // still be visible, as its own "Teardown" row, not silently dropped.
        use crate::data_model::Phase;
        use crate::metrics::PhaseBoundary;
        use chrono::TimeZone;

        let mut run = sample_run();
        let t0 = chrono::Utc.with_ymd_and_hms(2026, 9, 5, 9, 0, 0).unwrap();
        run.manifest.phase_boundaries = vec![
            PhaseBoundary {
                phase: Phase::Load,
                started_at: t0,
            },
            PhaseBoundary {
                phase: Phase::Drain,
                started_at: t0 + chrono::Duration::seconds(5),
            },
        ];
        // Drain's own work finishes 10s after it starts...
        run.manifest.load_and_drain_completed_at = Some(t0 + chrono::Duration::seconds(15));
        // ...but teardown (stopping the Z3 stack) takes a further 20s before
        // run_completed_at is set.
        run.manifest.run_completed_at = Some(t0 + chrono::Duration::seconds(35));

        let md = render_report(&[run]);
        let section = &md[md.find("## Setup phase timing").unwrap()..];
        let section = &section[..section.find("## RPC compatibility matrix").unwrap()];

        assert!(
            section.contains("Drain") && section.contains("| 10s |"),
            "Drain row must show 10s (load_and_drain_completed_at - Drain.started_at), \
             not 30s (run_completed_at - Drain.started_at); got:\n{section}"
        );
        assert!(
            section.contains("Teardown") && section.contains("| 20s |"),
            "the residual gap to run_completed_at must appear as its own Teardown row; \
             got:\n{section}"
        );
    }

    #[test]
    fn setup_phase_unlisted_method_is_not_silently_dropped() {
        // Regression guard: an off-roster method exercised only during
        // setup (never during Load/Drain) must still surface somewhere in
        // the report — in the setup-phase appendix's own unlisted listing —
        // rather than becoming invisible once the headline matrix's
        // "Observed outside the tracked roster" table was scoped to
        // Load/Drain only.
        let mut run = sample_run();
        run.rpc_calls.push(RpcCall {
            call_id: "c-setup-unlisted".into(),
            run_id: "r".into(),
            method: "z_getbalanceforaccount".into(),
            backend: crate::data_model::Backend::Zallet,
            params_hash: None,
            request_at: chrono::Utc::now(),
            response_at: Some(chrono::Utc::now()),
            latency_ms: Some(6),
            success: true,
            error_code: None,
            error_message: None,
            phase: Phase::Funding,
        });
        let md = render_report(&[run]);
        assert!(
            md.contains("Also observed outside the tracked roster during setup"),
            "expected the setup-phase unlisted subsection; got:\n{md}"
        );
        assert!(md.contains("z_getbal"));
    }

    #[test]
    fn render_table_pads_separator_width_to_content() {
        let mut md = String::new();
        render_table(
            &["Short", "Also short"],
            &[vec![
                "a-very-long-cell-value-here".to_string(),
                "x".to_string(),
            ]],
            &mut md,
        );
        let sep_line = md.lines().nth(1).unwrap();
        let cols: Vec<&str> = sep_line.trim_matches('|').split('|').collect();
        assert!(
            cols[0].len() > cols[1].len(),
            "long-content column must get a wider separator: {sep_line}"
        );
    }

    #[test]
    fn breakable_inserts_zero_width_space_every_n_chars() {
        let out = breakable("abcdefghij", 4);
        assert_eq!(out, "abcd\u{200B}efgh\u{200B}ij");
    }

    #[test]
    fn breakable_short_string_is_unchanged() {
        let out = breakable("abc", 8);
        assert_eq!(out, "abc");
    }

    #[test]
    fn render_report_with_assets_embeds_chart_images() {
        let dir = tempfile::tempdir().unwrap();
        let md = render_report_with_assets(&[sample_run()], dir.path());
        assert!(md.contains(".png"));
        assert!(md.contains("![RPC calls per second over time]"));
        assert!(md.contains("![Latency over time]"));
    }

    #[test]
    fn render_report_with_assets_creates_directory() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path().join("nested_assets");
        assert!(!assets.exists());
        render_report_with_assets(&[sample_run()], &assets);
        assert!(assets.exists());
    }
}
