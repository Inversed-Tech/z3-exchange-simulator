//! CLI argument parsing and subcommand dispatch.
//!
//! Parses top-level flags and subcommands, then hands control to the
//! appropriate scenario runner or utility command.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use tokio_util::sync::CancellationToken;

use crate::scenarios::runner::run as runner_run;
use crate::scenarios::runner::{
    load_scenario, validate_scenario, ConfigError, LoadShape, RunOptions, RunnerError,
};
use crate::synthetic::{write_fixtures, AccountGenerator, FixtureError, GeneratorError};

// ── Cli types ─────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "z3sim",
    version,
    about = "Z3 Exchange Simulator — exchange-scale load testing for the Zcash Z3 stack",
    long_about = "Z3 Exchange Simulator — exchange-scale load testing for the Zcash Z3 stack.\n\n\
                  Verbosity is controlled by --verbose / --quiet. \
                  For fine-grained log filtering, set RUST_LOG before invoking:\n\n  \
                  RUST_LOG=z3_exchange_simulator=debug z3sim run --scenario configs/scenarios/smoke.yaml"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
    #[arg(short = 'v', long, global = true, conflicts_with = "quiet")]
    pub verbose: bool,
    #[arg(short = 'q', long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    Run(RunArgs),
    GenerateFixtures(GenerateFixturesArgs),
    ValidateScenario { path: PathBuf },
}

#[derive(clap::Args)]
pub struct RunArgs {
    /// Scenario YAML file to execute
    #[arg(long)]
    pub scenario: PathBuf,
    /// Validate and summarise without starting Z3
    #[arg(long)]
    pub dry_run: bool,
    /// Load profile [steady|ramp|burst|mixed]
    #[arg(long, value_enum, default_value_t = LoadShapeArg::Steady)]
    pub load_shape: LoadShapeArg,
    /// Ramp duration in seconds (ignored unless --load-shape ramp)
    #[arg(long, default_value_t = 60)]
    pub ramp_secs: u64,
    /// Steady phase before burst spike (ignored unless --load-shape burst)
    #[arg(long, default_value_t = 60)]
    pub burst_pre_secs: u64,
    /// Duration of burst spike (ignored unless --load-shape burst)
    #[arg(long, default_value_t = 30)]
    pub burst_secs: u64,
    /// TPS spike multiplier (ignored unless --load-shape burst)
    #[arg(long, default_value_t = 3.0)]
    pub burst_multiplier: f64,
    /// Max concurrent in-flight transactions
    #[arg(long, default_value_t = 64)]
    pub max_in_flight: usize,
    /// Base directory for run output
    #[arg(long, default_value = "experiments/runs")]
    pub output_base: PathBuf,
    /// Reuse an existing Zallet hot wallet from a prior run
    #[arg(long)]
    pub hot_wallet_uuid: Option<String>,
}

#[derive(clap::Args)]
pub struct GenerateFixturesArgs {
    /// Scenario YAML to derive seed and account params from
    #[arg(long)]
    pub scenario: PathBuf,
    /// Directory to write accounts.json and wallets.json
    #[arg(long)]
    pub out: PathBuf,
}

#[derive(ValueEnum, Clone, Default, Debug)]
pub enum LoadShapeArg {
    #[default]
    Steady,
    Ramp,
    Burst,
    Mixed,
}

// ── CliError ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CliError {
    Scenario(ConfigError),
    Run(RunnerError),
    Fixture(FixtureError),
    Generator(GeneratorError),
    /// Bad flag combination (e.g. --burst-multiplier <= 0); distinct from Io.
    InvalidArgs(String),
    /// Actual filesystem errors not covered by Scenario/Fixture variants.
    Io(std::io::Error),
    /// SIGINT received; teardown completed; triggers exit 130.
    Interrupted,
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // ValidationErrors needs special formatting: count prefix + per-field lines.
            CliError::Scenario(ConfigError::ValidationErrors(errs)) => {
                write!(f, "scenario validation failed ({} error(s)):", errs.len())?;
                for (field, msg) in errs {
                    write!(f, "\n  {field}: {msg}")?;
                }
                Ok(())
            }
            CliError::Scenario(e) => write!(f, "{e}"),
            CliError::Run(e) => write!(f, "{e}"),
            CliError::Fixture(e) => write!(f, "{e}"),
            CliError::Generator(e) => write!(f, "{e}"),
            CliError::InvalidArgs(s) => write!(f, "invalid arguments: {s}"),
            CliError::Io(e) => write!(f, "{e}"),
            CliError::Interrupted => write!(f, "interrupted"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CliError::Scenario(e) => Some(e),
            CliError::Run(e) => Some(e),
            CliError::Fixture(e) => Some(e),
            CliError::Generator(e) => Some(e),
            CliError::Io(e) => Some(e),
            _ => None,
        }
    }
}

// ── init_tracing ──────────────────────────────────────────────────────────────

pub fn init_tracing(verbose: bool, quiet: bool) {
    use std::io::IsTerminal as _;

    let level = if verbose {
        tracing::Level::DEBUG
    } else if quiet {
        tracing::Level::ERROR
    } else {
        tracing::Level::WARN
    };

    // with_default_directive sets the fallback level; from_env_lossy() reads RUST_LOG
    // and overrides the default when set. from_env_lossy (not from_default_env) is
    // required so that with_default_directive actually takes effect when RUST_LOG is unset.
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(level.into())
        .from_env_lossy();

    tracing_subscriber::fmt()
        .compact()
        .with_env_filter(filter)
        .with_target(true)
        .with_level(true)
        .with_ansi(std::io::stderr().is_terminal())
        .with_writer(std::io::stderr)
        .init();
}

// ── build_load_shape ──────────────────────────────────────────────────────────

fn build_load_shape(args: &RunArgs) -> Result<LoadShape, CliError> {
    match args.load_shape {
        LoadShapeArg::Steady => Ok(LoadShape::SteadyState),
        LoadShapeArg::Ramp => Ok(LoadShape::Ramp {
            ramp_secs: args.ramp_secs,
        }),
        LoadShapeArg::Burst => {
            if args.burst_multiplier <= 0.0 {
                return Err(CliError::InvalidArgs(
                    "--burst-multiplier must be > 0.0".into(),
                ));
            }
            Ok(LoadShape::Burst {
                pre_burst_secs: args.burst_pre_secs,
                burst_secs: args.burst_secs,
                spike_multiplier: args.burst_multiplier,
            })
        }
        LoadShapeArg::Mixed => Ok(LoadShape::Mixed),
    }
}

// ── validate_scenario_command ─────────────────────────────────────────────────

fn validate_scenario_command(path: &Path) -> Result<(), CliError> {
    let config = load_scenario(path).map_err(CliError::Scenario)?;
    validate_scenario(&config).map_err(CliError::Scenario)?;
    tracing::debug!("scenario loaded and validated");
    println!("OK: {} is valid", path.display());
    println!("  name : {}", config.name);
    println!("  seed : {}", config.seed);
    println!("  hash : {}", config.config_hash);
    Ok(())
}

// ── generate_fixtures_command ─────────────────────────────────────────────────

fn generate_fixtures_command(args: &GenerateFixturesArgs) -> Result<(), CliError> {
    let config = load_scenario(&args.scenario).map_err(CliError::Scenario)?;
    validate_scenario(&config).map_err(CliError::Scenario)?;
    tracing::debug!("scenario loaded and validated");
    let mut gen = AccountGenerator::new(config).map_err(CliError::Generator)?;
    let population = gen.generate_population().map_err(CliError::Generator)?;
    write_fixtures(&population, &args.out).map_err(CliError::Fixture)?;
    println!("Accounts : {}", population.accounts.len());
    println!("Active   : {}", population.active_count());
    println!("Written  : {}", args.out.display());
    println!("  accounts.json");
    println!("  wallets.json");
    Ok(())
}

// ── run_command ───────────────────────────────────────────────────────────────

async fn run_command(
    args: RunArgs,
    cancel_override: Option<CancellationToken>,
) -> Result<(), CliError> {
    let config = load_scenario(&args.scenario).map_err(CliError::Scenario)?;
    validate_scenario(&config).map_err(CliError::Scenario)?;
    tracing::debug!("scenario loaded and validated");

    let load_shape = build_load_shape(&args)?;

    if args.dry_run {
        let opts = RunOptions {
            output_base: args.output_base.clone(),
            load_shape,
            max_in_flight: args.max_in_flight,
            dry_run: true,
            hot_wallet_uuid: args.hot_wallet_uuid.clone(),
            cancel: None,
            ..RunOptions::default() // covers polling: None only
        };
        runner_run(config, opts).await.map_err(CliError::Run)?;
        return Ok(());
    }

    // NOTE: cancellation is only checked in the load phase scheduler loop.
    // Ctrl-C during setup or warmup will be registered but will not take
    // effect until the load phase begins. See lifecycle.rs for the phases
    // that would need token checks to improve responsiveness here.
    let token = cancel_override.unwrap_or_default();
    let cancel_clone = token.clone();
    let ctrl_c_handle = tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::warn!("interrupt signal received; waiting for current phase to complete");
        cancel_clone.cancel();
    });

    eprintln!("Starting run — press Ctrl-C to interrupt");

    let opts = RunOptions {
        output_base: args.output_base.clone(),
        load_shape,
        max_in_flight: args.max_in_flight,
        dry_run: false,
        hot_wallet_uuid: args.hot_wallet_uuid.clone(),
        cancel: Some(token.clone()),
        ..RunOptions::default() // covers polling: None only
    };
    let result = runner_run(config, opts).await;
    ctrl_c_handle.abort();

    if token.is_cancelled() {
        if let Err(e) = &result {
            tracing::error!("teardown error during cancellation: {e}");
        }
        return Err(CliError::Interrupted);
    }

    let r = result.map_err(CliError::Run)?;
    println!("Run ID   : {}", r.run_id);
    if let Some(dir) = &r.output_dir {
        println!("Output   : {}", dir.display());
    }
    println!("Attempted: {}", r.stats.total_attempted);
    println!("Confirmed: {}", r.stats.confirmed);
    println!("Failed   : {}", r.stats.failed);
    println!("Timed out: {}", r.stats.timed_out);
    Ok(())
}

// ── dispatch ──────────────────────────────────────────────────────────────────

pub async fn dispatch(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Commands::Run(args) => run_command(args, None).await,
        Commands::GenerateFixtures(args) => generate_fixtures_command(&args),
        Commands::ValidateScenario { path } => validate_scenario_command(&path),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    fn valid_scenario_yaml() -> NamedTempFile {
        // Raw byte string preserves indentation exactly (line-continuation eats spaces).
        let yaml = br#"
name: test-cli
description: CLI test scenario
seed: 42
accounts_count: 10
accounts_active_fraction: 0.5
load_duration_seconds: 60
load_target_tps: 1.0
flows:
  transparent_to_transparent: 1.0
  transparent_to_shielded: 0.0
  shielded_to_transparent: 0.0
  shielded_to_shielded: 0.0
activity_profiles:
  low_fraction: 0.50
  medium_fraction: 0.35
  high_fraction: 0.15
amounts:
  min_zatoshis: 10000
  max_zatoshis: 10000000
confirmations_deposit_required: 3
observability:
  record_rpc_calls: true
  record_component_logs: true
  metric_sampling_interval_secs: 5
  mempool_saturation_threshold: 500
"#;
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(yaml).unwrap();
        f
    }

    fn dry_run_args(scenario: PathBuf, output_base: PathBuf) -> RunArgs {
        RunArgs {
            scenario,
            dry_run: true,
            load_shape: LoadShapeArg::Steady,
            ramp_secs: 60,
            burst_pre_secs: 60,
            burst_secs: 30,
            burst_multiplier: 3.0,
            max_in_flight: 64,
            output_base,
            hot_wallet_uuid: None,
        }
    }

    fn live_run_args(scenario: PathBuf, output_base: PathBuf) -> RunArgs {
        RunArgs {
            scenario,
            dry_run: false,
            load_shape: LoadShapeArg::Steady,
            ramp_secs: 60,
            burst_pre_secs: 60,
            burst_secs: 30,
            burst_multiplier: 3.0,
            max_in_flight: 64,
            output_base,
            hot_wallet_uuid: None,
        }
    }

    // ── Section 10.1: argument parsing ──────────────────────────────────────────

    #[test]
    fn run_subcommand_parses_scenario_and_dry_run() {
        let cli =
            Cli::try_parse_from(["z3sim", "run", "--scenario", "foo.yaml", "--dry-run"]).unwrap();
        if let Commands::Run(args) = cli.command {
            assert_eq!(args.scenario, PathBuf::from("foo.yaml"));
            assert!(args.dry_run);
        } else {
            panic!("expected Run command");
        }
    }

    #[test]
    fn run_subcommand_parses_hot_wallet_uuid() {
        let cli = Cli::try_parse_from([
            "z3sim",
            "run",
            "--scenario",
            "foo.yaml",
            "--hot-wallet-uuid",
            "abc123",
        ])
        .unwrap();
        if let Commands::Run(args) = cli.command {
            assert_eq!(args.hot_wallet_uuid, Some("abc123".to_string()));
        } else {
            panic!("expected Run command");
        }
    }

    #[test]
    fn run_subcommand_hot_wallet_uuid_defaults_to_none() {
        let cli = Cli::try_parse_from(["z3sim", "run", "--scenario", "foo.yaml"]).unwrap();
        if let Commands::Run(args) = cli.command {
            assert!(args.hot_wallet_uuid.is_none());
        } else {
            panic!("expected Run command");
        }
    }

    #[test]
    fn generate_fixtures_requires_both_scenario_and_out() {
        let missing_out =
            Cli::try_parse_from(["z3sim", "generate-fixtures", "--scenario", "foo.yaml"]);
        assert!(missing_out.is_err(), "expected error when --out is missing");

        let missing_scenario =
            Cli::try_parse_from(["z3sim", "generate-fixtures", "--out", "/tmp/out"]);
        assert!(
            missing_scenario.is_err(),
            "expected error when --scenario is missing"
        );
    }

    #[test]
    fn validate_scenario_accepts_positional() {
        let cli = Cli::try_parse_from(["z3sim", "validate-scenario", "foo.yaml"]).unwrap();
        if let Commands::ValidateScenario { path } = cli.command {
            assert_eq!(path, PathBuf::from("foo.yaml"));
        } else {
            panic!("expected ValidateScenario command");
        }
    }

    #[test]
    fn verbose_and_quiet_are_mutually_exclusive() {
        let result =
            Cli::try_parse_from(["z3sim", "--verbose", "--quiet", "validate-scenario", "x"]);
        assert!(
            result.is_err(),
            "expected error when both --verbose and --quiet are passed"
        );
    }

    #[test]
    fn unknown_subcommand_returns_error() {
        let result = Cli::try_parse_from(["z3sim", "unknown"]);
        assert!(result.is_err());
    }

    #[test]
    fn missing_required_scenario_returns_error() {
        let result = Cli::try_parse_from(["z3sim", "run"]);
        assert!(result.is_err());
    }

    #[test]
    fn load_shape_accepts_all_four_variants() {
        for shape in ["steady", "ramp", "burst", "mixed"] {
            let cli = Cli::try_parse_from([
                "z3sim",
                "run",
                "--scenario",
                "foo.yaml",
                "--load-shape",
                shape,
            ])
            .unwrap_or_else(|e| panic!("failed to parse --load-shape {shape}: {e}"));
            assert!(matches!(cli.command, Commands::Run(_)));
        }
    }

    #[test]
    fn invalid_load_shape_returns_error() {
        let result = Cli::try_parse_from([
            "z3sim",
            "run",
            "--scenario",
            "foo.yaml",
            "--load-shape",
            "invalid",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn load_shape_ramp_converts_with_default_ramp_secs() {
        let cli = Cli::try_parse_from([
            "z3sim",
            "run",
            "--scenario",
            "foo.yaml",
            "--load-shape",
            "ramp",
        ])
        .unwrap();
        if let Commands::Run(args) = cli.command {
            let shape = build_load_shape(&args).unwrap();
            assert!(matches!(shape, LoadShape::Ramp { ramp_secs: 60 }));
        }
    }

    #[test]
    fn load_shape_burst_converts_with_all_parameters() {
        let cli = Cli::try_parse_from([
            "z3sim",
            "run",
            "--scenario",
            "foo.yaml",
            "--load-shape",
            "burst",
            "--burst-pre-secs",
            "90",
            "--burst-secs",
            "20",
            "--burst-multiplier",
            "5.0",
        ])
        .unwrap();
        if let Commands::Run(args) = cli.command {
            let shape = build_load_shape(&args).unwrap();
            if let LoadShape::Burst {
                pre_burst_secs,
                burst_secs,
                spike_multiplier,
            } = shape
            {
                assert_eq!(pre_burst_secs, 90);
                assert_eq!(burst_secs, 20);
                assert!(
                    (spike_multiplier - 5.0).abs() < 1e-9,
                    "spike_multiplier: {spike_multiplier}"
                );
            } else {
                panic!("expected Burst shape");
            }
        }
    }

    #[test]
    fn burst_multiplier_negative_produces_invalid_args() {
        // Clap rejects negative numbers passed as space-separated flag values ("-1.0" looks like
        // a short flag). Construct RunArgs directly to test build_load_shape in isolation.
        let args = RunArgs {
            scenario: PathBuf::from("foo.yaml"),
            dry_run: false,
            load_shape: LoadShapeArg::Burst,
            ramp_secs: 60,
            burst_pre_secs: 60,
            burst_secs: 30,
            burst_multiplier: -1.0,
            max_in_flight: 64,
            output_base: PathBuf::from("experiments/runs"),
            hot_wallet_uuid: None,
        };
        let err = build_load_shape(&args).unwrap_err();
        assert!(
            matches!(err, CliError::InvalidArgs(_)),
            "expected InvalidArgs, got: {err:?}"
        );
    }

    #[test]
    fn burst_multiplier_zero_produces_invalid_args() {
        let cli = Cli::try_parse_from([
            "z3sim",
            "run",
            "--scenario",
            "foo.yaml",
            "--load-shape",
            "burst",
            "--burst-multiplier",
            "0.0",
        ])
        .unwrap();
        if let Commands::Run(args) = cli.command {
            let err = build_load_shape(&args).unwrap_err();
            assert!(
                matches!(err, CliError::InvalidArgs(_)),
                "expected InvalidArgs, got: {err:?}"
            );
        }
    }

    #[test]
    fn load_shape_ramp_converts_ramp_secs_correctly() {
        let cli = Cli::try_parse_from([
            "z3sim",
            "run",
            "--scenario",
            "foo.yaml",
            "--load-shape",
            "ramp",
            "--ramp-secs",
            "120",
        ])
        .unwrap();
        if let Commands::Run(args) = cli.command {
            let shape = build_load_shape(&args).unwrap();
            assert!(matches!(shape, LoadShape::Ramp { ramp_secs: 120 }));
        }
    }

    #[test]
    fn load_shape_burst_spike_multiplier_converts() {
        let cli = Cli::try_parse_from([
            "z3sim",
            "run",
            "--scenario",
            "foo.yaml",
            "--load-shape",
            "burst",
            "--burst-multiplier",
            "7.5",
        ])
        .unwrap();
        if let Commands::Run(args) = cli.command {
            let shape = build_load_shape(&args).unwrap();
            if let LoadShape::Burst {
                spike_multiplier, ..
            } = shape
            {
                assert!(
                    (spike_multiplier - 7.5).abs() < 1e-9,
                    "spike_multiplier: {spike_multiplier}"
                );
            } else {
                panic!("expected Burst shape");
            }
        }
    }

    #[test]
    fn invalid_args_display_starts_with_invalid_arguments() {
        let err = CliError::InvalidArgs("--burst-multiplier must be > 0.0".into());
        let s = err.to_string();
        assert!(
            s.starts_with("invalid arguments:"),
            "expected 'invalid arguments:' prefix, got: {s}"
        );
    }

    #[test]
    fn round_trip_max_in_flight() {
        let cli = Cli::try_parse_from([
            "z3sim",
            "run",
            "--scenario",
            "foo.yaml",
            "--max-in-flight",
            "128",
        ])
        .unwrap();
        if let Commands::Run(args) = cli.command {
            assert_eq!(args.max_in_flight, 128);
        }
    }

    #[test]
    fn round_trip_output_base() {
        let cli = Cli::try_parse_from([
            "z3sim",
            "run",
            "--scenario",
            "foo.yaml",
            "--output-base",
            "/custom/dir",
        ])
        .unwrap();
        if let Commands::Run(args) = cli.command {
            assert_eq!(args.output_base, PathBuf::from("/custom/dir"));
        }
    }

    #[test]
    fn round_trip_hot_wallet_uuid_some() {
        let cli = Cli::try_parse_from([
            "z3sim",
            "run",
            "--scenario",
            "foo.yaml",
            "--hot-wallet-uuid",
            "abc123",
        ])
        .unwrap();
        if let Commands::Run(args) = cli.command {
            assert_eq!(args.hot_wallet_uuid, Some("abc123".to_string()));
        }
    }

    #[test]
    fn round_trip_hot_wallet_uuid_none() {
        let cli = Cli::try_parse_from(["z3sim", "run", "--scenario", "foo.yaml"]).unwrap();
        if let Commands::Run(args) = cli.command {
            assert!(args.hot_wallet_uuid.is_none());
        }
    }

    // ── Section 10.2: validate_scenario_command ─────────────────────────────────

    #[test]
    fn validate_scenario_cmd_valid_yaml_returns_ok() {
        let f = valid_scenario_yaml();
        let result = validate_scenario_command(f.path());
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    #[test]
    fn validate_scenario_cmd_nonexistent_path_returns_io_error() {
        let result = validate_scenario_command(Path::new("/nonexistent/path/scenario.yaml"));
        assert!(
            matches!(result, Err(CliError::Scenario(ConfigError::Io(_)))),
            "got: {result:?}"
        );
    }

    #[test]
    fn validate_scenario_cmd_malformed_yaml_returns_parse_error() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"key: [unclosed bracket").unwrap();
        let result = validate_scenario_command(f.path());
        assert!(
            matches!(result, Err(CliError::Scenario(ConfigError::Parse(_)))),
            "got: {result:?}"
        );
    }

    #[test]
    fn validate_scenario_cmd_invalid_flows_returns_validation_errors() {
        // flows sum to 0.5, not 1.0 — triggers ValidationErrors.
        let yaml = br#"
name: bad-flows
description: test
seed: 1
accounts_count: 10
accounts_active_fraction: 0.5
load_duration_seconds: 60
load_target_tps: 1.0
flows:
  transparent_to_transparent: 0.5
  transparent_to_shielded: 0.0
  shielded_to_transparent: 0.0
  shielded_to_shielded: 0.0
activity_profiles:
  low_fraction: 0.50
  medium_fraction: 0.35
  high_fraction: 0.15
amounts:
  min_zatoshis: 10000
  max_zatoshis: 10000000
confirmations_deposit_required: 3
observability:
  record_rpc_calls: true
  record_component_logs: true
  metric_sampling_interval_secs: 5
  mempool_saturation_threshold: 500
"#;
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(yaml).unwrap();
        let result = validate_scenario_command(f.path());
        assert!(
            matches!(
                result,
                Err(CliError::Scenario(ConfigError::ValidationErrors(_)))
            ),
            "got: {result:?}"
        );
    }

    #[test]
    fn cli_error_display_includes_error_count() {
        let errs = vec![(
            "flows".to_string(),
            "flow fractions must sum to 1.0, got 0.500000".to_string(),
        )];
        let err = CliError::Scenario(ConfigError::ValidationErrors(errs));
        let s = err.to_string();
        assert!(s.contains("(1 error(s)):"), "got: {s}");
    }

    #[test]
    fn cli_error_display_validation_errors_indented() {
        let errs = vec![
            ("load_target_tps".to_string(), "must be > 0.0".to_string()),
            (
                "flows".to_string(),
                "flow fractions must sum to 1.0, got 0.500000".to_string(),
            ),
        ];
        let err = CliError::Scenario(ConfigError::ValidationErrors(errs));
        let s = err.to_string();
        assert!(s.contains("(2 error(s)):"), "got: {s}");
        assert!(
            s.contains("  load_target_tps: must be > 0.0"),
            "missing field line, got: {s}"
        );
        assert!(s.contains("  flows:"), "missing flows line, got: {s}");
    }

    // ── Section 10.3: generate_fixtures_command ──────────────────────────────────

    #[test]
    fn generate_fixtures_cmd_creates_accounts_and_wallets_json() {
        let f = valid_scenario_yaml();
        let out = TempDir::new().unwrap();
        let args = GenerateFixturesArgs {
            scenario: f.path().to_path_buf(),
            out: out.path().to_path_buf(),
        };
        generate_fixtures_command(&args).unwrap();
        assert!(out.path().join("accounts.json").exists());
        assert!(out.path().join("wallets.json").exists());
    }

    #[test]
    fn generate_fixtures_cmd_creates_output_dir() {
        let f = valid_scenario_yaml();
        let base = TempDir::new().unwrap();
        let out_path = base.path().join("nested").join("fixtures");
        let args = GenerateFixturesArgs {
            scenario: f.path().to_path_buf(),
            out: out_path.clone(),
        };
        generate_fixtures_command(&args).unwrap();
        assert!(out_path.join("accounts.json").exists());
    }

    #[test]
    fn generate_fixtures_cmd_deterministic_account_ids() {
        let f = valid_scenario_yaml();
        let out1 = TempDir::new().unwrap();
        let out2 = TempDir::new().unwrap();
        generate_fixtures_command(&GenerateFixturesArgs {
            scenario: f.path().to_path_buf(),
            out: out1.path().to_path_buf(),
        })
        .unwrap();
        generate_fixtures_command(&GenerateFixturesArgs {
            scenario: f.path().to_path_buf(),
            out: out2.path().to_path_buf(),
        })
        .unwrap();
        let a1: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(out1.path().join("accounts.json")).unwrap(),
        )
        .unwrap();
        let a2: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(out2.path().join("accounts.json")).unwrap(),
        )
        .unwrap();
        let ids1: Vec<&str> = a1
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["account_id"].as_str().unwrap())
            .collect();
        let ids2: Vec<&str> = a2
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["account_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids1, ids2, "account_ids differ across seeded runs");
    }

    #[test]
    fn generate_fixtures_cmd_account_count_matches_scenario() {
        let f = valid_scenario_yaml(); // accounts_count: 10
        let out = TempDir::new().unwrap();
        let args = GenerateFixturesArgs {
            scenario: f.path().to_path_buf(),
            out: out.path().to_path_buf(),
        };
        generate_fixtures_command(&args).unwrap();
        let content = std::fs::read_to_string(out.path().join("accounts.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(
            parsed.as_array().unwrap().len(),
            10,
            "expected 10 accounts (matches scenario accounts_count)"
        );
    }

    #[test]
    fn generate_fixtures_cmd_nonexistent_scenario_returns_error() {
        let args = GenerateFixturesArgs {
            scenario: PathBuf::from("/nonexistent/path/scenario.yaml"),
            out: PathBuf::from("/tmp/out"),
        };
        let result = generate_fixtures_command(&args);
        assert!(
            matches!(result, Err(CliError::Scenario(_))),
            "got: {result:?}"
        );
    }

    // ── Section 10.4: dry-run tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn run_cmd_dry_run_valid_scenario_returns_ok() {
        let f = valid_scenario_yaml();
        let tmp = TempDir::new().unwrap();
        let args = dry_run_args(f.path().to_path_buf(), tmp.path().to_path_buf());
        let result = run_command(args, None).await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    #[tokio::test]
    async fn run_cmd_dry_run_creates_no_run_directory() {
        let f = valid_scenario_yaml();
        let tmp = TempDir::new().unwrap();
        let args = dry_run_args(f.path().to_path_buf(), tmp.path().to_path_buf());
        run_command(args, None).await.unwrap();
        let entries: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
        assert!(
            entries.is_empty(),
            "expected empty output_base dir after dry-run, found {} entries",
            entries.len()
        );
    }

    #[tokio::test]
    async fn run_cmd_dry_run_invalid_scenario_returns_error() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"invalid: [unclosed").unwrap();
        let tmp = TempDir::new().unwrap();
        let args = dry_run_args(f.path().to_path_buf(), tmp.path().to_path_buf());
        let result = run_command(args, None).await;
        assert!(
            matches!(result, Err(CliError::Scenario(_))),
            "got: {result:?}"
        );
    }

    // ── Section 10.1 (cont.): build_load_shape coverage for Steady and Mixed ───

    #[test]
    fn build_load_shape_steady_returns_steady_state() {
        let tmp = TempDir::new().unwrap();
        // dry_run_args defaults to LoadShapeArg::Steady
        let args = dry_run_args(PathBuf::from("foo.yaml"), tmp.path().to_path_buf());
        let shape = build_load_shape(&args).unwrap();
        assert!(
            matches!(shape, LoadShape::SteadyState),
            "expected SteadyState, got: {shape:?}"
        );
    }

    #[test]
    fn build_load_shape_mixed_returns_mixed() {
        let tmp = TempDir::new().unwrap();
        let args = RunArgs {
            load_shape: LoadShapeArg::Mixed,
            ..dry_run_args(PathBuf::from("foo.yaml"), tmp.path().to_path_buf())
        };
        let shape = build_load_shape(&args).unwrap();
        assert!(
            matches!(shape, LoadShape::Mixed),
            "expected Mixed, got: {shape:?}"
        );
    }

    // ── CliError::Interrupted display ────────────────────────────────────────────

    #[test]
    fn cli_error_interrupted_display() {
        assert_eq!(CliError::Interrupted.to_string(), "interrupted");
    }

    // ── Section 10.6: cancellation ───────────────────────────────────────────────

    #[tokio::test]
    async fn run_cmd_cancelled_token_returns_interrupted() {
        let f = valid_scenario_yaml();
        let tmp = TempDir::new().unwrap();
        let token = CancellationToken::new();
        token.cancel(); // pre-cancel: token.is_cancelled() is true from the start
        let args = live_run_args(f.path().to_path_buf(), tmp.path().to_path_buf());
        // runner_run() will fail at setup (no Z3 stack in CI), but token.is_cancelled()
        // is checked after runner_run() returns regardless — Interrupted takes precedence.
        let result = run_command(args, Some(token)).await;
        assert!(
            matches!(result, Err(CliError::Interrupted)),
            "expected Interrupted, got: {result:?}"
        );
    }
}
