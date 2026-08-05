//! Renders the aggregated run data, RPC matrix, load curves, and candidate
//! findings into a single Markdown document. Candidate findings are
//! rendered as flagged evidence with a mechanical severity tier, not
//! finished prose — see `findings.rs` for why root-cause authorship is a
//! deliberately separate step from this pipeline.

use std::path::Path;

use super::charts::{render_latency_chart, render_tps_chart};
use super::findings::{
    flag_candidates, Finding, FindingCategory, Severity, HIGH_DISPARITY_GAP, HIGH_RATE,
    HIGH_TAIL_LATENCY_MULTIPLE, MEDIUM_RATE, MIN_ABSOLUTE_P99_MS, MIN_DISPARITY_GAP,
    MIN_TAIL_LATENCY_SAMPLE, TAIL_LATENCY_MULTIPLE,
};
use super::load_curve::{
    windowed_load_curve, DEFAULT_WINDOW_SECS, DEGRADATION_ERROR_RATE,
    DEGRADATION_HIGH_LATENCY_MULTIPLE, DEGRADATION_LATENCY_MULTIPLE, MIN_WINDOW_CALLS,
};
use super::loader::RunData;
use super::rpc_matrix::{build_matrix, Category};

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
        let mut v: Vec<&str> = runs.iter().map(|r| r.manifest.scenario_name.as_str()).collect();
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

    let high = findings.iter().filter(|f| f.severity == Severity::High).count();
    let medium = findings.iter().filter(|f| f.severity == Severity::Medium).count();
    let low = findings.iter().filter(|f| f.severity == Severity::Low).count();
    md.push_str(&format!(
        "- **Candidate findings:** **{high} High**, {medium} Medium, {low} Low (see \
         \"Candidate findings\" below; tier definitions in the Appendix)\n\n"
    ));

    let mut high_findings: Vec<&Finding> = findings.iter().filter(|f| f.severity == Severity::High).collect();
    high_findings.sort_by(|a, b| a.category.to_string().cmp(&b.category.to_string()).then(a.summary.cmp(&b.summary)));
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
    let headers = ["Run ID", "Scenario", "Target TPS", "Zebra", "Zaino", "Zallet", "Simulator commit"];
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
            let confirmed = run.intents.iter().filter(|i| i.outcome == "confirmed").count();
            let failed = run.intents.iter().filter(|i| i.outcome == "failed").count();
            let timed_out = run.intents.iter().filter(|i| i.outcome == "timed_out").count();
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

/// Per-run TPS/latency/error-rate curve, all RPC methods combined — the
/// "load-curve results (TPS vs. latency)" scope.md asks for. Rendered per
/// run, not aggregated, since two runs' time series only make sense
/// side-by-side if they share a load shape (see `load_curve.rs`). When
/// `assets_dir` is provided, a TPS chart and a latency chart are rendered
/// as PNGs and embedded after each run's table.
fn render_load_curve(runs: &[RunData], assets_dir: Option<&Path>, md: &mut String) {
    md.push_str("## Load curve by run\n\n");
    md.push_str(&format!(
        "Achieved throughput, latency, and error rate in fixed {DEFAULT_WINDOW_SECS}-second \
         windows across each run's own timeline — all RPC methods combined (see the RPC \
         compatibility matrix below for per-method detail). Candidate inflection points \
         detected from these curves appear under \"Load degradation\" in Candidate findings.\n\n"
    ));
    let mut any = false;
    for run in runs {
        let points = windowed_load_curve(&run.rpc_calls, DEFAULT_WINDOW_SECS);
        if points.is_empty() {
            continue;
        }
        any = true;
        md.push_str(&format!("### {}\n\n", run.manifest.run_id));

        if let Some(dir) = assets_dir {
            if let Ok(path) = render_tps_chart(dir, &run.manifest.run_id, &points) {
                md.push_str(&format!("![TPS over time]({})\n\n", path.display()));
            }
            if let Ok(path) = render_latency_chart(dir, &run.manifest.run_id, &points) {
                md.push_str(&format!("![Latency over time]({})\n\n", path.display()));
            }
        }

        let headers = ["+s", "Calls", "Errors", "TPS", "P50 ms", "P95 ms", "P99 ms"];
        let start = points[0].window_start;
        let fmt_ms = |v: Option<f64>| v.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into());
        let rows: Vec<Vec<String>> = points
            .iter()
            .map(|p| {
                let offset = (p.window_start - start).num_seconds();
                vec![
                    format!("+{offset}s"),
                    p.calls.to_string(),
                    p.errors.to_string(),
                    format!("{:.1}", p.tps),
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

fn render_rpc_matrix(runs: &[RunData], md: &mut String) {
    md.push_str("## RPC compatibility matrix\n\n");
    md.push_str(
        "Derived mechanically from the `rpc_calls.jsonl` of every run listed above. \
         `Not tested` means no observed call, not \"known to fail\" — see \
         docs/rpc/rpc-coverage-matrix.md for zcashd-equivalence and parity notes, \
         which this table does not attempt to reproduce.\n\n",
    );
    let matrix = build_matrix(runs);
    let headers = [
        "Method", "Backend", "Status", "Calls", "Successes", "P50 ms", "P95 ms", "P99 ms",
        "Error codes",
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
                ]
            })
            .collect();
        render_table(&headers, &rows, md);
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
        "- **Latency outlier** — a method is flagged at all only when its P99 reaches \
         `{:.0}x` its P50 *and* P99 is at least `{:.0}ms`, over at least `{}` calls (below \
         that, tail latency is treated as noise). Once flagged: `>= {:.0}x` P50 = **High**, \
         otherwise **Medium**.\n",
        TAIL_LATENCY_MULTIPLE, MIN_ABSOLUTE_P99_MS, MIN_TAIL_LATENCY_SAMPLE, HIGH_TAIL_LATENCY_MULTIPLE,
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
    let mut md = String::new();
    md.push_str("# Z3 Exchange Simulator — Findings Report\n\n");
    render_executive_summary(runs, &findings, &mut md);
    render_run_metadata(runs, &mut md);
    render_load_results(runs, &mut md);
    render_load_curve(runs, assets_dir, &mut md);
    render_rpc_matrix(runs, &mut md);
    render_findings(&findings, &mut md);
    render_limitations(runs, &mut md);
    render_severity_appendix(&mut md);
    md
}

/// Renders the full Markdown report for the given runs, without charts.
pub fn render_report(runs: &[RunData]) -> String {
    render_report_impl(runs, None)
}

/// Renders the full Markdown report, additionally generating a TPS chart
/// and a latency chart per run into `assets_dir` (created if missing) and
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
        assert!(md.contains("TPS"));
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
        assert!(md.contains("**[High]**") || md.contains("**[Medium]**") || md.contains("**[Low]**"));
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
    fn render_table_pads_separator_width_to_content() {
        let mut md = String::new();
        render_table(
            &["Short", "Also short"],
            &[vec!["a-very-long-cell-value-here".to_string(), "x".to_string()]],
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
        assert!(md.contains("![TPS over time]"));
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
