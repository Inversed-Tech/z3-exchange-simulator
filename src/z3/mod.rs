use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio::time::{self, MissedTickBehavior};

use crate::data_model::MetricSample;
use crate::metrics::MetricsRecorder;

pub mod contract;
pub mod env_id;
pub mod run_lock;

use contract::{ContractError, Z3Contract};
use env_id::compose_project_for_env;

const SERVICES: &[&str] = &["zebra", "zallet", "zaino"];
const ENV_FILE: &str = ".env.regtest";

/// Default RPC Router credentials for the regtest stack (see z3-contract.yaml
/// `rpc_auth.credential_env_vars`; the compose defaults are `zebra` / `zebra`).
const DEFAULT_REGTEST_RPC_USER: &str = "zebra";
const DEFAULT_REGTEST_RPC_PASSWORD: &str = "zebra";

/// Returns the host to use for RPC connections. Defaults to `127.0.0.1` but
/// can be overridden by `Z3_RPC_HOST` — useful when running from inside a
/// devcontainer where the Docker daemon is on the macOS host (use
/// `host.docker.internal`).
fn rpc_host() -> String {
    std::env::var("Z3_RPC_HOST").unwrap_or_else(|_| "127.0.0.1".into())
}

/// The `docker compose` interpolation overrides (host ports + subnet/static
/// IP) for a given, already-derived port set and subnet assignment, keyed by
/// the exact variable names `docker-compose.regtest.yml`/
/// `docker-compose.regtest.override.yml` reference. Passed as extra process
/// environment variables on every `docker compose` invocation (see
/// [`Z3Stack::run_compose`]) — Compose's own variable-interpolation
/// precedence puts the invoking process's environment above `--env-file`, so
/// this takes effect without ever rewriting the shared `.env.regtest` file.
/// That matters because `.env.regtest` lives in the single, shared
/// `external/z3` checkout: two environments (e.g. a stable run and a
/// concurrent `--fresh-env` one) mutating the same file would race, but each
/// has its own independent process environment.
///
/// Covers every host port `docker compose config` actually publishes for the
/// regtest stack — confirmed against its resolved output, not just the ports
/// referenced elsewhere in this codebase by name: two of the six
/// (`Z3_ZAINO_HOST_GRPC_PORT`, `Z3_ZEBRA_HOST_HEALTH_PORT`) have no other
/// caller and are easy to miss, and a missing entry here means two
/// environments collide on that specific host port with no error until
/// `docker compose up` fails to bind it.
fn compose_env_overrides(
    ports: &env_id::PortSet,
    subnet: &env_id::SubnetAssignment,
) -> Vec<(String, String)> {
    vec![
        (
            "Z3_ZEBRA_HOST_RPC_PORT".to_string(),
            ports.zebra_rpc.to_string(),
        ),
        (
            "Z3_ZEBRA_HOST_HEALTH_PORT".to_string(),
            ports.zebra_health.to_string(),
        ),
        (
            "Z3_ZAINO_HOST_GRPC_PORT".to_string(),
            ports.zaino_grpc.to_string(),
        ),
        (
            "Z3_ZAINO_HOST_JSON_RPC_PORT".to_string(),
            ports.zaino_json_rpc.to_string(),
        ),
        (
            "Z3_ZALLET_HOST_RPC_PORT".to_string(),
            ports.zallet_rpc.to_string(),
        ),
        (
            "Z3_REGTEST_RPC_ROUTER_HOST_PORT".to_string(),
            ports.rpc_router.to_string(),
        ),
        ("Z3_SIM_SUBNET".to_string(), subnet.subnet.clone()),
        ("Z3_SIM_ZAINO_IP".to_string(), subnet.zaino_ip.clone()),
    ]
}

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Z3Config {
    /// Path to the cloned Z3 Docker Compose repository (`external/z3`).
    pub compose_dir: PathBuf,
    /// RPC Router URL — all simulator RPC calls go here.
    pub rpc_url: String,
    /// HTTP Basic Auth credentials for the RPC Router (regtest: `zebra`/`zebra`).
    pub basic_auth: Option<(String, String)>,
    /// Docker Compose project name (e.g. `z3-regtest`), used to scope `docker stats`.
    pub compose_project: String,
    /// Directory to write per-service log files into.
    pub log_dir: PathBuf,
    /// Run ID written into resource metric samples.
    pub run_id: String,
    /// How long to wait for `getblockchaininfo` to succeed before giving up.
    pub health_check_timeout_secs: u64,
    /// How often to poll `docker stats` for CPU/memory samples.
    pub resource_sample_interval_secs: u64,
    /// Extra process environment variables passed on every `docker compose`
    /// invocation — the per-`env_id` derived host ports and subnet/static IP
    /// (see [`compose_env_overrides`]). Empty for [`Z3Config::from_contract`]
    /// configs (mainnet/testnet), which have no env-id-based isolation.
    pub compose_env_overrides: Vec<(String, String)>,
}

impl Z3Config {
    /// Regtest defaults for a given environment identity (see
    /// [`env_id::resolve_env_id`]). Ports/credentials match the values in
    /// `z3-contract.yaml`; prefer [`Z3Config::from_contract`] to derive them
    /// from the checked-out Z3 contract rather than relying on these
    /// constants.
    ///
    /// `compose_project` is derived from `env_id` so two environments never
    /// collide on Compose project, network, or volume names. `rpc_url` and
    /// `compose_env_overrides` are both derived from the SAME `env_id`-keyed
    /// port/subnet formulas (see `env_id::derive_ports`/`derive_subnet`), so
    /// the host ports Docker actually binds and the port the simulator's own
    /// RPC client connects to can never independently drift — one formula,
    /// not two hardcoded values.
    ///
    /// `compose_dir` is a parameter (not hardcoded) so callers — in
    /// particular tests exercising the surrounding setup/CLI logic without a
    /// real `external/z3` checkout — can point it at a path that is
    /// guaranteed not to exist, which [`Z3Config::check_preconditions`]
    /// turns into a fast, side-effect-free failure before anything in this
    /// module touches the filesystem or Docker.
    ///
    /// Errors only if `env_id` is not the well-formed id
    /// [`env_id::resolve_env_id`] always produces.
    pub fn for_run(
        run_id: &str,
        log_dir: PathBuf,
        env_id: &str,
        compose_dir: PathBuf,
    ) -> Result<Self, Z3Error> {
        let ports = env_id::derive_ports(env_id)?;
        let subnet = env_id::derive_subnet(env_id)?;
        Ok(Self {
            compose_dir,
            rpc_url: format!("http://{}:{}", rpc_host(), ports.rpc_router),
            basic_auth: Some((
                DEFAULT_REGTEST_RPC_USER.into(),
                DEFAULT_REGTEST_RPC_PASSWORD.into(),
            )),
            compose_project: compose_project_for_env(env_id),
            log_dir,
            run_id: run_id.into(),
            health_check_timeout_secs: 180,
            resource_sample_interval_secs: 5,
            compose_env_overrides: compose_env_overrides(&ports, &subnet),
        })
    }

    /// Build a config by reading `z3-contract.yaml` from the Z3 compose directory,
    /// deriving the RPC endpoint, compose project, and auth for the given network
    /// (e.g. `"regtest"`). Credentials are read from the contract's named env vars,
    /// falling back to the documented regtest defaults.
    pub fn from_contract(
        compose_dir: PathBuf,
        network: &str,
        run_id: &str,
        log_dir: PathBuf,
    ) -> Result<Self, ContractError> {
        let contract = Z3Contract::from_compose_dir(&compose_dir)?;
        let net = contract.network(network)?;

        let rpc_url = net.primary_rpc_url(&rpc_host())?;
        let basic_auth = if net.uses_username_password_auth() {
            let (user, pass) = match &net.rpc_auth.credential_env_vars {
                Some(vars) => (
                    std::env::var(&vars.user)
                        .unwrap_or_else(|_| DEFAULT_REGTEST_RPC_USER.to_string()),
                    std::env::var(&vars.password)
                        .unwrap_or_else(|_| DEFAULT_REGTEST_RPC_PASSWORD.to_string()),
                ),
                None => (
                    DEFAULT_REGTEST_RPC_USER.to_string(),
                    DEFAULT_REGTEST_RPC_PASSWORD.to_string(),
                ),
            };
            Some((user, pass))
        } else {
            // Cookie-auth networks (mainnet/testnet) — basic auth not used.
            None
        };

        Ok(Self {
            compose_dir,
            rpc_url,
            basic_auth,
            compose_project: net.compose_project.clone(),
            log_dir,
            run_id: run_id.into(),
            health_check_timeout_secs: 180,
            resource_sample_interval_secs: 5,
            compose_env_overrides: Vec::new(),
        })
    }

    /// Cheap, side-effect-free check that `compose_dir` and its `.env.regtest`
    /// exist, before anything else in this module touches the filesystem or
    /// shells out to Docker. Deliberately callable on a bare `Z3Config` (not
    /// just via [`Z3Stack::start`]) so [`Z3Config::ensure_wallet_bootstrapped`]
    /// can run it first — that function mutates `.env.regtest` and runs real
    /// bootstrap scripts, so it must never be reached for a `compose_dir` that
    /// was never cloned/configured (a fresh checkout, CI, or a test pointed at
    /// a deliberately nonexistent path).
    pub fn check_preconditions(&self) -> Result<(), Z3Error> {
        if !self.compose_dir.exists() {
            return Err(Z3Error::ComposeDirNotFound(self.compose_dir.clone()));
        }
        let env = self.compose_dir.join(ENV_FILE);
        if !env.exists() {
            return Err(Z3Error::EnvFileNotFound(env));
        }
        Ok(())
    }

    /// Write this run's Compose project name and host ports into
    /// `.env.regtest`, in place. Necessary because `regtest-init.sh` (sources
    /// the file directly) and `regtest-miner-setup.sh` (greps it) predate
    /// per-environment identity and read the file's own values rather than
    /// accepting the `-p`/process-env overrides `Z3Stack`'s own `docker
    /// compose` invocations use — without this, they always operate on
    /// whatever project/ports the file's checked-in defaults name, never
    /// this run's resolved `env_id`. Idempotent: replaces an existing `KEY=`
    /// line in place, appends if missing. Skips `Z3_SIM_SUBNET`/
    /// `Z3_SIM_ZAINO_IP` — neither script reads them; the transient
    /// containers `regtest-init.sh` itself may briefly start get the correct
    /// subnet from `compose_env_overrides`, passed as process env alongside
    /// it (see [`Z3Config::ensure_wallet_bootstrapped`]).
    fn sync_bootstrap_env_file(&self) -> Result<(), Z3Error> {
        let env_file = self.compose_dir.join(ENV_FILE);
        let contents = std::fs::read_to_string(&env_file)
            .map_err(|_| Z3Error::EnvFileNotFound(env_file.clone()))?;

        let mut pairs = vec![(
            "COMPOSE_PROJECT_NAME".to_string(),
            self.compose_project.clone(),
        )];
        pairs.extend(
            self.compose_env_overrides
                .iter()
                .filter(|(k, _)| k != "Z3_SIM_SUBNET" && k != "Z3_SIM_ZAINO_IP")
                .cloned(),
        );

        let mut lines: Vec<String> = contents.lines().map(str::to_string).collect();
        for (key, value) in &pairs {
            let prefix = format!("{key}=");
            match lines.iter_mut().find(|l| l.starts_with(&prefix)) {
                Some(line) => *line = format!("{key}={value}"),
                None => lines.push(format!("{key}={value}")),
            }
        }
        let mut new_contents = lines.join("\n");
        new_contents.push('\n');
        std::fs::write(&env_file, new_contents).map_err(Z3Error::EnvFileSync)
    }

    /// Ensure this run's Compose project has an initialized wallet before
    /// `Z3Stack::start()` brings it up. A freshly-derived `env_id` names a
    /// brand-new, empty Compose project — nothing else in this module's
    /// environment-identity wiring initializes its wallet, since that is
    /// `regtest-init.sh`'s (mnemonic + `hot_wallet` account) and
    /// `regtest-miner-setup.sh`'s (miner address) job, and neither script
    /// can be pointed at a specific project via `-p`/process-env alone (see
    /// [`Z3Config::sync_bootstrap_env_file`]). Both scripts are already
    /// idempotent (`regtest-init.sh` checks the target volume for an
    /// existing wallet before doing anything; `regtest-miner-setup.sh`
    /// checks whether the miner address is still the shipped placeholder),
    /// so this is cheap on every call after the first for a given `env_id`.
    ///
    /// This and the two scripts it runs all read/write the single
    /// `.env.regtest` shared by every environment on this checkout, so the
    /// write-then-run-scripts sequence below is serialized across
    /// environments via [`run_lock::acquire_bootstrap_lock`] — otherwise a
    /// `--fresh-env` run bootstrapping concurrently with another run's own
    /// first-ever bootstrap could interleave writes to that file and end up
    /// creating its Docker resources under the OTHER environment's project
    /// name. The lock is held only for this function's duration, not the
    /// whole run, so it costs at most a short wait once per environment.
    pub async fn ensure_wallet_bootstrapped(&self) -> Result<(), Z3Error> {
        self.check_preconditions()?;

        // `File::lock` blocks the calling thread until acquired; run it on a
        // blocking-pool thread so it never stalls the async runtime.
        let compose_dir = self.compose_dir.clone();
        let _bootstrap_lock =
            tokio::task::spawn_blocking(move || run_lock::acquire_bootstrap_lock(&compose_dir))
                .await
                .map_err(|e| Z3Error::RunLockIo(std::io::Error::other(e.to_string())))??;

        self.sync_bootstrap_env_file()?;

        let init_script = self.compose_dir.join("scripts").join("regtest-init.sh");
        run_bootstrap_script(&init_script, &self.compose_env_overrides).await?;

        let miner_setup_script = PathBuf::from("scripts/dev/regtest-miner-setup.sh");
        run_bootstrap_script(&miner_setup_script, &self.compose_env_overrides).await?;

        Ok(())
    }
}

/// Run a bootstrap shell script (`regtest-init.sh`/`regtest-miner-setup.sh`)
/// with `env_overrides` set on its process environment (inherited by any
/// `docker compose` it shells out to internally), surfacing a non-zero exit
/// or spawn failure as a descriptive [`Z3Error::BootstrapScript`].
async fn run_bootstrap_script(
    script: &Path,
    env_overrides: &[(String, String)],
) -> Result<(), Z3Error> {
    let output = Command::new("bash")
        .arg(script)
        .envs(env_overrides.iter().cloned())
        .output()
        .await
        .map_err(|e| Z3Error::BootstrapScript {
            script: script.to_path_buf(),
            stderr: e.to_string(),
        })?;

    if !output.status.success() {
        return Err(Z3Error::BootstrapScript {
            script: script.to_path_buf(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum Z3Error {
    ComposeDirNotFound(PathBuf),
    EnvFileNotFound(PathBuf),
    LogDirCreate(std::io::Error),
    ComposeCommand {
        args: String,
        stderr: String,
    },
    HealthCheckTimeout {
        after_secs: u64,
    },
    /// Failed to read or write the cached environment id at
    /// `configs/local/env-id` (see `env_id::resolve_env_id`).
    EnvIdCacheIo(std::io::Error),
    /// An `env_id` string did not match the expected 8-character lowercase
    /// hex format.
    InvalidEnvId(String),
    /// Failed to open or create the per-`env_id` lock file itself (distinct
    /// from `EnvironmentBusy`, which means the file opened fine but another
    /// process already holds the lock).
    RunLockIo(std::io::Error),
    /// Another `z3sim run` already holds the advisory lock for this
    /// environment. Stable `env_id`s are per-checkout, so two concurrent
    /// invocations against the same checkout resolve the same one; use
    /// `--fresh-env` to run a second, independent environment concurrently.
    EnvironmentBusy {
        env_id: String,
        lock_path: PathBuf,
    },
    /// Failed to write the derived project name/ports into `.env.regtest`
    /// (see [`Z3Config::sync_bootstrap_env_file`]).
    EnvFileSync(std::io::Error),
    /// `regtest-init.sh` or `regtest-miner-setup.sh` (see
    /// [`Z3Config::ensure_wallet_bootstrapped`]) failed to spawn or exited
    /// non-zero.
    BootstrapScript {
        script: PathBuf,
        stderr: String,
    },
}

impl std::fmt::Display for Z3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Z3Error::ComposeDirNotFound(p) => write!(
                f,
                "Z3 compose directory not found: {}. Run `make clone-z3` first.",
                p.display()
            ),
            Z3Error::EnvFileNotFound(p) => write!(
                f,
                "Env file not found: {}. Run `./scripts/regtest-init.sh` in the Z3 directory first.",
                p.display()
            ),
            Z3Error::LogDirCreate(e) => write!(f, "Failed to create log directory: {}", e),
            Z3Error::ComposeCommand { args, stderr } => {
                write!(f, "`docker compose {}` failed: {}", args, stderr)
            }
            Z3Error::HealthCheckTimeout { after_secs } => write!(
                f,
                "Z3 stack did not respond to getblockchaininfo within {}s",
                after_secs
            ),
            Z3Error::EnvIdCacheIo(e) => {
                write!(f, "failed to read/write the environment id cache: {e}")
            }
            Z3Error::InvalidEnvId(id) => {
                write!(f, "invalid environment id {id:?}: expected 8-character lowercase hex")
            }
            Z3Error::RunLockIo(e) => write!(f, "failed to open the environment lock file: {e}"),
            Z3Error::EnvironmentBusy { env_id, lock_path } => write!(
                f,
                "environment {env_id} is already in use by another run (lock held at {}) — \
                 pass --fresh-env to start an independent, non-colliding environment",
                lock_path.display()
            ),
            Z3Error::EnvFileSync(e) => write!(f, "failed to update .env.regtest: {e}"),
            Z3Error::BootstrapScript { script, stderr } => {
                write!(f, "`{}` failed: {stderr}", script.display())
            }
        }
    }
}

impl std::error::Error for Z3Error {}

// ── Z3Stack ───────────────────────────────────────────────────────────────────

pub struct Z3Stack {
    config: Z3Config,
    background_tasks: Vec<JoinHandle<()>>,
    metrics: Option<Arc<dyn MetricsRecorder>>,
}

impl Z3Stack {
    pub fn new(config: Z3Config, metrics: Option<Arc<dyn MetricsRecorder>>) -> Self {
        Self {
            config,
            background_tasks: Vec::new(),
            metrics,
        }
    }

    /// Start the Docker Compose stack, wait for the RPC Router to become healthy,
    /// then launch background log capture and resource sampling tasks.
    pub async fn start(&mut self) -> Result<(), Z3Error> {
        self.bring_up().await?;
        self.spawn_log_capture();
        self.spawn_resource_sampling();
        Ok(())
    }

    /// Bring the Compose stack up and wait for the RPC Router to become
    /// healthy, without spawning the log-capture/resource-sampling background
    /// tasks a full scenario run keeps alive for its own duration.
    ///
    /// Used by `z3sim print-versions` (bootstrap's version-printing step),
    /// which is a short-lived process: `spawn_log_capture`'s tasks each hold
    /// a `docker compose logs --follow` child process open for as long as
    /// they run, and are only ever reaped by `stop()`'s explicit
    /// `handle.abort()` — a caller that starts the stack, prints versions,
    /// and exits (rather than running a full scenario and calling `stop()`)
    /// would otherwise leak those child processes as orphans.
    pub async fn bring_up(&mut self) -> Result<(), Z3Error> {
        self.check_preconditions()?;
        tokio::fs::create_dir_all(&self.config.log_dir)
            .await
            .map_err(Z3Error::LogDirCreate)?;
        self.run_compose(&["up", "-d"]).await?;
        self.wait_until_ready().await?;
        Ok(())
    }

    /// Resolve the image (repository:tag) and image ID Docker actually used
    /// for each of [`VERSION_SERVICES`] in this stack's Compose project.
    ///
    /// Requires the project to already have created containers (i.e.
    /// [`Z3Stack::start`]/[`Z3Stack::bring_up`] has run at least once) —
    /// `docker compose images` reports on containers, not on compose config
    /// alone, and returns nothing for a project that has never been brought
    /// up.
    pub async fn image_digests(&self) -> Result<Vec<ImageInfo>, Z3Error> {
        let args = ["images", "--format", "json"];
        let full_args = compose_base_args(&self.config.compose_project, &args);
        let output = Command::new("docker")
            .args(&full_args)
            .envs(self.config.compose_env_overrides.iter().cloned())
            .current_dir(&self.config.compose_dir)
            .output()
            .await
            .map_err(|e| Z3Error::ComposeCommand {
                args: args.join(" "),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(Z3Error::ComposeCommand {
                args: args.join(" "),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        parse_compose_images(
            &String::from_utf8_lossy(&output.stdout),
            &self.config.compose_project,
        )
    }

    /// Abort background tasks and bring the Docker Compose stack down.
    ///
    /// Must be called explicitly before dropping — `Drop` aborts background
    /// tasks but cannot run the async `docker compose down`.
    pub async fn stop(&mut self) -> Result<(), Z3Error> {
        for handle in self.background_tasks.drain(..) {
            handle.abort();
        }
        self.run_compose(&["down"]).await
    }

    fn check_preconditions(&self) -> Result<(), Z3Error> {
        self.config.check_preconditions()
    }

    async fn run_compose(&self, args: &[&str]) -> Result<(), Z3Error> {
        let full_args = compose_base_args(&self.config.compose_project, args);
        let output = Command::new("docker")
            .args(&full_args)
            .envs(self.config.compose_env_overrides.iter().cloned())
            .current_dir(&self.config.compose_dir)
            .output()
            .await
            .map_err(|e| Z3Error::ComposeCommand {
                args: args.join(" "),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(Z3Error::ComposeCommand {
                args: args.join(" "),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(())
    }

    async fn wait_until_ready(&self) -> Result<(), Z3Error> {
        let client = reqwest::Client::new();
        let timeout = self.config.health_check_timeout_secs;
        let deadline = time::Instant::now() + Duration::from_secs(timeout);
        // rpc-router can be briefly UP during a Docker restart loop; require
        // 3 consecutive successes so we only proceed when it is truly stable.
        let mut consecutive = 0u32;
        const REQUIRED: u32 = 3;

        loop {
            if time::Instant::now() >= deadline {
                return Err(Z3Error::HealthCheckTimeout {
                    after_secs: timeout,
                });
            }
            if health_check(
                &client,
                &self.config.rpc_url,
                self.config.basic_auth.as_ref(),
            )
            .await
            {
                consecutive += 1;
                if consecutive >= REQUIRED {
                    return Ok(());
                }
            } else {
                consecutive = 0;
            }
            time::sleep(Duration::from_millis(500)).await;
        }
    }

    fn spawn_log_capture(&mut self) {
        for &service in SERVICES {
            let log_path = self.config.log_dir.join(format!("{}.log", service));
            let compose_dir = self.config.compose_dir.clone();
            let project = self.config.compose_project.clone();
            let env_overrides = self.config.compose_env_overrides.clone();
            let svc = service.to_string();
            self.background_tasks.push(tokio::spawn(async move {
                capture_logs(&compose_dir, &project, &env_overrides, &svc, &log_path).await;
            }));
        }
    }

    fn spawn_resource_sampling(&mut self) {
        let interval = self.config.resource_sample_interval_secs;
        let run_id = self.config.run_id.clone();
        let project = self.config.compose_project.clone();
        let metrics = self.metrics.clone();
        self.background_tasks.push(tokio::spawn(async move {
            sample_resources(run_id, project, interval, metrics).await;
        }));
    }
}

impl Drop for Z3Stack {
    fn drop(&mut self) {
        for handle in &self.background_tasks {
            handle.abort();
        }
    }
}

// ── Free functions ────────────────────────────────────────────────────────────

/// The `docker compose` argument list shared by every invocation this crate
/// makes: the env file plus the `-p` project flag (which takes precedence
/// over any `COMPOSE_PROJECT_NAME` value baked into `.env.regtest`, so the
/// derived per-environment project name is authoritative regardless of
/// whether `.env.regtest` was ever rewritten for it — see
/// `env_id::compose_project_for_env`), followed by the operation-specific
/// arguments. Split out as a pure function so tests can assert on the
/// constructed argument list without invoking Docker.
fn compose_base_args(project: &str, args: &[&str]) -> Vec<String> {
    let mut full = vec![
        "compose".to_string(),
        "--env-file".to_string(),
        ENV_FILE.to_string(),
        "-p".to_string(),
        project.to_string(),
    ];
    full.extend(args.iter().map(|s| s.to_string()));
    full
}

async fn health_check(
    client: &reqwest::Client,
    rpc_url: &str,
    auth: Option<&(String, String)>,
) -> bool {
    let body = serde_json::json!({
        "method": "getblockchaininfo",
        "params": [],
        "id": 1
    });

    let mut req = client
        .post(rpc_url)
        .json(&body)
        .timeout(Duration::from_secs(2));
    if let Some((user, pass)) = auth {
        req = req.basic_auth(user, Some(pass));
    }

    let Ok(resp) = req.send().await else {
        return false;
    };

    let Ok(json) = resp.json::<serde_json::Value>().await else {
        return false;
    };

    // Zebra implements regtest as Network::Testnet(Regtest) so getblockchaininfo
    // returns "test", not "regtest". Accept both to guard against mainnet ("main").
    json.pointer("/result/chain")
        .and_then(|v| v.as_str())
        .map(|chain| chain == "regtest" || chain == "test")
        .unwrap_or(false)
}

async fn capture_logs(
    compose_dir: &Path,
    project: &str,
    env_overrides: &[(String, String)],
    service: &str,
    log_path: &Path,
) {
    let Ok(file) = tokio::fs::File::create(log_path).await else {
        return;
    };
    let mut writer = tokio::io::BufWriter::new(file);

    let full_args = compose_base_args(project, &["logs", "--follow", "--no-log-prefix", service]);
    let Ok(mut child) = Command::new("docker")
        .args(&full_args)
        .envs(env_overrides.iter().cloned())
        .current_dir(compose_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };

    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = writer.write_all(line.as_bytes()).await;
            let _ = writer.write_all(b"\n").await;
            let _ = writer.flush().await;
        }
    }
}

/// One component's resolved image identity, from `docker compose images
/// --format json` for a given, already-running Compose project.
///
/// Labeled `id`, not `digest`, deliberately: `docker compose images`'s own
/// `ID` field is the local content-addressed image ID, which is not
/// guaranteed to equal a pullable registry manifest digest (storage-driver
/// dependent) — and a locally-built image (Zallet, the RPC Router: neither is
/// ever pushed to a registry) has no registry digest to compare against at
/// all. It is still a unique, reproducible content hash of the image bytes
/// actually running, which is what "prove which image bytes ran" needs, just
/// not the same claim a registry digest would be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageInfo {
    pub service: String,
    pub image: String,
    pub id: String,
}

/// Services [`parse_compose_images`] reports on, in print/manifest order.
/// Deliberately distinct from [`SERVICES`] (log-capture/`docker stats` scope:
/// zebra/zallet/zaino only, matched against every sampled container) — this
/// additionally covers the RPC Router, which has its own image identity worth
/// surfacing here but no log-capture or resource-sampling role of its own.
const VERSION_SERVICES: &[&str] = &["zebra", "zaino", "zallet", "rpc-router"];

/// Parse `docker compose images --format json`'s output for `project`,
/// keeping only [`VERSION_SERVICES`] — this drops the stack's one-shot
/// permission/setup helper containers (`cookie-permissions`,
/// `zallet-permissions`), which `docker compose images` reports on too but
/// which have no image identity a reader would want surfaced here — and
/// deriving each kept container's service name by stripping the
/// `<project>-` prefix (mirrors [`container_in_project`]) and the trailing
/// Compose replica index (`-<n>`).
///
/// Returns an empty list, not an error, for the literal JSON `null` `docker
/// compose images` prints when the project has no created containers yet
/// (i.e. before `up -d` has ever run for it) — an absent stack is not a
/// parse failure.
fn parse_compose_images(json: &str, project: &str) -> Result<Vec<ImageInfo>, Z3Error> {
    #[derive(Deserialize)]
    struct RawImage {
        #[serde(rename = "ID")]
        id: String,
        #[serde(rename = "ContainerName")]
        container_name: String,
        #[serde(rename = "Repository")]
        repository: String,
        #[serde(rename = "Tag")]
        tag: String,
    }

    let trimmed = json.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return Ok(Vec::new());
    }
    let raw: Vec<RawImage> =
        serde_json::from_str(trimmed).map_err(|e| Z3Error::ComposeCommand {
            args: "images --format json".to_string(),
            stderr: format!("could not parse `docker compose images` output: {e}"),
        })?;

    let prefix = format!("{project}-");
    let mut out = Vec::new();
    for img in raw {
        let Some(rest) = img.container_name.strip_prefix(&prefix) else {
            continue;
        };
        let service = rest.rsplit_once('-').map_or(rest, |(svc, _n)| svc);
        if !VERSION_SERVICES.contains(&service) {
            continue;
        }
        out.push(ImageInfo {
            service: service.to_string(),
            image: format!("{}:{}", img.repository, img.tag),
            id: img.id,
        });
    }
    out.sort_by_key(|i| {
        VERSION_SERVICES
            .iter()
            .position(|s| *s == i.service)
            .unwrap_or(usize::MAX)
    });
    Ok(out)
}

/// Whether a `docker stats` container name belongs to the given Compose project.
///
/// Compose v2 names containers `<project>-<service>-<n>`, so we match on the
/// `<project>-` prefix. The trailing hyphen is significant: it stops a project
/// named `z3` from matching `z3-regtest-…`, and keeps the per-network projects
/// (`z3-mainnet` / `z3-testnet` / `z3-regtest`) from capturing each other's
/// containers.
fn container_in_project(container_name: &str, compose_project: &str) -> bool {
    container_name.starts_with(&format!("{compose_project}-"))
}

async fn sample_resources(
    run_id: String,
    compose_project: String,
    interval_secs: u64,
    metrics: Option<Arc<dyn MetricsRecorder>>,
) {
    let Some(m) = metrics else { return };
    let mut ticker = time::interval(Duration::from_secs(interval_secs));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;

        let Ok(output) = Command::new("docker")
            .args(["stats", "--no-stream", "--format", "{{json .}}"])
            .output()
            .await
        else {
            continue;
        };

        let now = chrono::Utc::now();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(name) = v.get("Name").and_then(|n| n.as_str()) else {
                continue;
            };
            // Only sample containers belonging to this Z3 network's project.
            if !container_in_project(name, &compose_project) {
                continue;
            }
            let labels = HashMap::from([("process".to_string(), name.to_string())]);

            if let Some(cpu) = parse_cpu_percent(&v) {
                m.record_metric(MetricSample {
                    run_id: run_id.clone(),
                    timestamp: now,
                    metric_name: "process_cpu_percent".into(),
                    value: cpu,
                    labels: labels.clone(),
                });
            }
            if let Some(mem) = parse_mem_mb(&v) {
                m.record_metric(MetricSample {
                    run_id: run_id.clone(),
                    timestamp: now,
                    metric_name: "process_memory_mb".into(),
                    value: mem,
                    labels,
                });
            }
        }
    }
}

fn parse_cpu_percent(v: &serde_json::Value) -> Option<f64> {
    v.get("CPUPerc")?
        .as_str()?
        .trim_end_matches('%')
        .parse()
        .ok()
}

fn parse_mem_mb(v: &serde_json::Value) -> Option<f64> {
    let used = v.get("MemUsage")?.as_str()?.split(" / ").next()?;
    // Docker reports memory in B, kB, MiB, or GiB
    if let Some(n) = used.strip_suffix("GiB") {
        return n.trim().parse::<f64>().ok().map(|g| g * 1024.0);
    }
    if let Some(n) = used.strip_suffix("MiB") {
        return n.trim().parse().ok();
    }
    if let Some(n) = used.strip_suffix("kB") {
        return n.trim().parse::<f64>().ok().map(|k| k / 1024.0);
    }
    if let Some(n) = used.strip_suffix("B") {
        return n.trim().parse::<f64>().ok().map(|b| b / 1_048_576.0);
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cpu_percent_basic() {
        let v = serde_json::json!({"CPUPerc": "0.12%"});
        assert_eq!(parse_cpu_percent(&v), Some(0.12));
    }

    #[test]
    fn parse_cpu_percent_zero() {
        let v = serde_json::json!({"CPUPerc": "0.00%"});
        assert_eq!(parse_cpu_percent(&v), Some(0.0));
    }

    #[test]
    fn parse_mem_mb_mib() {
        let v = serde_json::json!({"MemUsage": "128MiB / 8GiB"});
        assert_eq!(parse_mem_mb(&v), Some(128.0));
    }

    #[test]
    fn parse_mem_mb_gib() {
        let v = serde_json::json!({"MemUsage": "2GiB / 8GiB"});
        assert_eq!(parse_mem_mb(&v), Some(2048.0));
    }

    #[test]
    fn parse_mem_mb_kb() {
        let v = serde_json::json!({"MemUsage": "512kB / 8GiB"});
        assert!((parse_mem_mb(&v).unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn parse_mem_mb_missing_field() {
        let v = serde_json::json!({});
        assert_eq!(parse_mem_mb(&v), None);
    }

    #[test]
    fn check_preconditions_missing_compose_dir() {
        let mut config = Z3Config::for_run(
            "test-run",
            PathBuf::from("/tmp/z3-test-logs"),
            "a1b2c3d4",
            PathBuf::from("external/z3"),
        )
        .unwrap();
        config.compose_dir = PathBuf::from("/tmp/z3-nonexistent-compose-dir-test");
        let stack = Z3Stack::new(config, None);
        assert!(matches!(
            stack.check_preconditions(),
            Err(Z3Error::ComposeDirNotFound(_))
        ));
    }

    // ── Z3Config::for_run defaults ────────────────────────────────────────────

    #[test]
    fn z3config_for_run_sets_correct_defaults() {
        let cfg = Z3Config::for_run(
            "run-42",
            PathBuf::from("/tmp/logs"),
            "a1b2c3d4",
            PathBuf::from("external/z3"),
        )
        .unwrap();
        assert_eq!(cfg.compose_dir, PathBuf::from("external/z3"));
        // rpc_url's port must be the SAME value derived for
        // Z3_REGTEST_RPC_ROUTER_HOST_PORT in compose_env_overrides below —
        // one formula, not two independently-hardcoded ports (see
        // Z3Config::for_run's doc comment).
        assert_eq!(cfg.rpc_url, "http://127.0.0.1:8340");
        assert_eq!(
            cfg.basic_auth,
            Some(("zebra".to_string(), "zebra".to_string()))
        );
        assert_eq!(cfg.compose_project, "z3-sim-a1b2c3d4");
        assert_eq!(cfg.health_check_timeout_secs, 180);
        assert_eq!(cfg.resource_sample_interval_secs, 5);
        assert_eq!(cfg.run_id, "run-42");
        assert_eq!(cfg.log_dir, PathBuf::from("/tmp/logs"));
        assert_eq!(
            cfg.compose_env_overrides,
            vec![
                ("Z3_ZEBRA_HOST_RPC_PORT".to_string(), "38340".to_string()),
                ("Z3_ZEBRA_HOST_HEALTH_PORT".to_string(), "48340".to_string()),
                ("Z3_ZAINO_HOST_GRPC_PORT".to_string(), "18340".to_string()),
                (
                    "Z3_ZAINO_HOST_JSON_RPC_PORT".to_string(),
                    "28340".to_string()
                ),
                ("Z3_ZALLET_HOST_RPC_PORT".to_string(), "58340".to_string()),
                (
                    "Z3_REGTEST_RPC_ROUTER_HOST_PORT".to_string(),
                    "8340".to_string()
                ),
                ("Z3_SIM_SUBNET".to_string(), "10.195.0.0/24".to_string()),
                ("Z3_SIM_ZAINO_IP".to_string(), "10.195.0.10".to_string()),
            ]
        );
    }

    #[test]
    fn z3config_for_run_rejects_invalid_env_id() {
        assert!(matches!(
            Z3Config::for_run(
                "run-1",
                PathBuf::from("/tmp/logs"),
                "not-hex!",
                PathBuf::from("external/z3"),
            ),
            Err(Z3Error::InvalidEnvId(_))
        ));
    }

    #[test]
    fn z3config_for_run_differs_by_env_id() {
        // Two distinct env_ids must never resolve to the same host ports,
        // subnet, or Compose project — the actual isolation guarantee this
        // track exists to provide, not merely distinct in-memory labels.
        let a = Z3Config::for_run(
            "run-1",
            PathBuf::from("/tmp/logs"),
            "a1b2c3d4",
            PathBuf::from("external/z3"),
        )
        .unwrap();
        let b = Z3Config::for_run(
            "run-1",
            PathBuf::from("/tmp/logs"),
            "00000001",
            PathBuf::from("external/z3"),
        )
        .unwrap();
        assert_ne!(a.compose_project, b.compose_project);
        assert_ne!(a.rpc_url, b.rpc_url);
        assert_ne!(a.compose_env_overrides, b.compose_env_overrides);
    }

    #[test]
    fn sync_bootstrap_env_file_replaces_existing_and_appends_missing_keys() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join(".env.regtest"),
            "COMPOSE_PROJECT_NAME=z3-regtest\nZ3_ZEBRA_IMAGE=foo\n",
        )
        .unwrap();

        let config = Z3Config::for_run(
            "run-1",
            PathBuf::from("/tmp/logs"),
            "a1b2c3d4",
            dir.path().to_path_buf(),
        )
        .unwrap();
        config.sync_bootstrap_env_file().unwrap();

        let contents = std::fs::read_to_string(dir.path().join(".env.regtest")).unwrap();
        // Existing key replaced in place, not duplicated.
        assert_eq!(
            contents.matches("COMPOSE_PROJECT_NAME=").count(),
            1,
            "{contents}"
        );
        assert!(contents.contains("COMPOSE_PROJECT_NAME=z3-sim-a1b2c3d4"));
        // Matches the same derivation z3config_for_run_sets_correct_defaults
        // asserts for this env_id.
        assert!(contents.contains("Z3_ZEBRA_HOST_RPC_PORT=38340"));
        assert!(contents.contains("Z3_ZEBRA_HOST_HEALTH_PORT=48340"));
        assert!(contents.contains("Z3_ZAINO_HOST_GRPC_PORT=18340"));
        assert!(contents.contains("Z3_ZAINO_HOST_JSON_RPC_PORT=28340"));
        assert!(contents.contains("Z3_ZALLET_HOST_RPC_PORT=58340"));
        assert!(contents.contains("Z3_REGTEST_RPC_ROUTER_HOST_PORT=8340"));
        // Untouched line preserved.
        assert!(contents.contains("Z3_ZEBRA_IMAGE=foo"));
        // Not written — neither regtest-init.sh nor regtest-miner-setup.sh
        // reads these; only docker compose interpolation needs them, and
        // that's already covered by compose_env_overrides / process env.
        assert!(!contents.contains("Z3_SIM_SUBNET"));
        assert!(!contents.contains("Z3_SIM_ZAINO_IP"));
    }

    #[test]
    fn sync_bootstrap_env_file_errors_when_env_file_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Z3Config::for_run(
            "run-1",
            PathBuf::from("/tmp/logs"),
            "a1b2c3d4",
            dir.path().to_path_buf(),
        )
        .unwrap();
        assert!(matches!(
            config.sync_bootstrap_env_file(),
            Err(Z3Error::EnvFileNotFound(_))
        ));
    }

    #[tokio::test]
    async fn run_bootstrap_script_surfaces_nonzero_exit_and_stderr() {
        let dir = tempfile::TempDir::new().unwrap();
        let script = dir.path().join("fail.sh");
        std::fs::write(&script, "#!/usr/bin/env bash\necho boom >&2\nexit 1\n").unwrap();

        let err = run_bootstrap_script(&script, &[]).await.unwrap_err();
        match err {
            Z3Error::BootstrapScript { script: s, stderr } => {
                assert_eq!(s, script);
                assert!(stderr.contains("boom"), "{stderr}");
            }
            other => panic!("expected BootstrapScript, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_bootstrap_script_passes_env_overrides_through() {
        let dir = tempfile::TempDir::new().unwrap();
        let script = dir.path().join("echo_env.sh");
        let out_file = dir.path().join("out.txt");
        std::fs::write(
            &script,
            format!(
                "#!/usr/bin/env bash\necho \"$MY_TEST_VAR\" > {}\n",
                out_file.display()
            ),
        )
        .unwrap();

        run_bootstrap_script(&script, &[("MY_TEST_VAR".to_string(), "hello".to_string())])
            .await
            .unwrap();

        assert_eq!(std::fs::read_to_string(&out_file).unwrap().trim(), "hello");
    }

    #[test]
    fn compose_base_args_passes_project_name_flag_for_every_call() {
        // Covers run_compose (up/down/etc.) and capture_logs (logs
        // --follow), which both build their Command from this helper — a
        // regression guard on the project-name wiring without invoking Docker.
        for args in [
            &["up", "-d"][..],
            &["down"][..],
            &["logs", "--follow", "--no-log-prefix", "zebra"][..],
        ] {
            let full = compose_base_args("z3-sim-deadbeef", args);
            let p_pos = full.iter().position(|a| a == "-p").expect("-p missing");
            assert_eq!(full[p_pos + 1], "z3-sim-deadbeef");
            let expected_tail: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            assert!(full.ends_with(&expected_tail));
        }
    }

    #[test]
    fn z3config_from_contract_derives_regtest_endpoint_and_auth() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join(contract::CONTRACT_FILENAME),
            r#"
contract_version: "1.0.0"
networks:
  regtest:
    z3_network: "Regtest"
    compose_project: "z3-regtest"
    external_network: "z3-regtest"
    rpc_auth:
      mode: username_password
      credential_env_vars:
        user: Z3_REGTEST_RPC_ROUTER_USER
        password: Z3_REGTEST_RPC_ROUTER_PASSWORD
    ports:
      rpc_router: {container: 8181, host: 8181}
      zaino_json_rpc: {container: 8237, host: 28237}
"#,
        )
        .unwrap();

        let cfg = Z3Config::from_contract(
            dir.path().to_path_buf(),
            "regtest",
            "run-1",
            PathBuf::from("/tmp/logs"),
        )
        .unwrap();

        assert_eq!(cfg.rpc_url, "http://127.0.0.1:8181");
        assert_eq!(cfg.compose_project, "z3-regtest");
        // Credentials fall back to the documented regtest defaults when the env
        // vars named by the contract are unset.
        assert_eq!(
            cfg.basic_auth,
            Some(("zebra".to_string(), "zebra".to_string()))
        );
        // Contract-driven configs have no env_id-based isolation.
        assert!(cfg.compose_env_overrides.is_empty());
    }

    // ── Z3Error Display ───────────────────────────────────────────────────────

    #[test]
    fn error_compose_dir_not_found_display_contains_path_and_hint() {
        let path = PathBuf::from("/opt/z3-exchange-simulator/external/z3");
        let msg = Z3Error::ComposeDirNotFound(path).to_string();
        assert!(
            msg.contains("/opt/z3-exchange-simulator/external/z3"),
            "path missing: {msg}"
        );
        assert!(
            msg.contains("make clone-z3"),
            "remediation hint missing: {msg}"
        );
    }

    #[test]
    fn error_env_file_not_found_display_contains_path_and_hint() {
        let path = PathBuf::from("/opt/z3/external/z3/.env.regtest");
        let msg = Z3Error::EnvFileNotFound(path).to_string();
        assert!(
            msg.contains("/opt/z3/external/z3/.env.regtest"),
            "path missing: {msg}"
        );
        assert!(
            msg.contains("regtest-init.sh"),
            "init script hint missing: {msg}"
        );
    }

    #[test]
    fn error_log_dir_create_display_contains_cause() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied");
        let msg = Z3Error::LogDirCreate(io_err).to_string();
        assert!(
            msg.contains("Failed to create log directory"),
            "context missing: {msg}"
        );
        assert!(msg.contains("permission denied"), "cause missing: {msg}");
    }

    #[test]
    fn error_compose_command_display_contains_args_and_stderr() {
        let msg = Z3Error::ComposeCommand {
            args: "up -d".into(),
            stderr: "no such service: foobar".into(),
        }
        .to_string();
        assert!(msg.contains("up -d"), "args missing: {msg}");
        assert!(
            msg.contains("no such service: foobar"),
            "stderr missing: {msg}"
        );
    }

    #[test]
    fn error_health_check_timeout_display_contains_duration() {
        let msg = Z3Error::HealthCheckTimeout { after_secs: 60 }.to_string();
        assert!(msg.contains("60"), "timeout duration missing: {msg}");
        assert!(
            msg.contains("getblockchaininfo"),
            "method name missing: {msg}"
        );
    }

    // ── check_preconditions filesystem states ─────────────────────────────────

    #[test]
    fn check_preconditions_compose_dir_present_env_file_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Z3Config {
            compose_dir: dir.path().to_path_buf(),
            rpc_url: "http://127.0.0.1:8181".into(),
            basic_auth: None,
            compose_project: "z3-regtest".into(),
            log_dir: PathBuf::from("/tmp/z3-test-logs"),
            run_id: "t".into(),
            health_check_timeout_secs: 180,
            resource_sample_interval_secs: 5,
            compose_env_overrides: Vec::new(),
        };
        let stack = Z3Stack::new(config, None);
        assert!(matches!(
            stack.check_preconditions(),
            Err(Z3Error::EnvFileNotFound(_))
        ));
    }

    #[test]
    fn check_preconditions_both_present_returns_ok() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join(".env.regtest"), "").unwrap();
        let config = Z3Config {
            compose_dir: dir.path().to_path_buf(),
            rpc_url: "http://127.0.0.1:8181".into(),
            basic_auth: None,
            compose_project: "z3-regtest".into(),
            log_dir: PathBuf::from("/tmp/z3-test-logs"),
            run_id: "t".into(),
            health_check_timeout_secs: 180,
            resource_sample_interval_secs: 5,
            compose_env_overrides: Vec::new(),
        };
        let stack = Z3Stack::new(config, None);
        assert!(stack.check_preconditions().is_ok());
    }

    // ── parse_cpu_percent ────────────────────────────────────────────────────

    // ── container_in_project (docker stats scoping) ───────────────────────────

    #[test]
    fn container_in_project_matches_own_services() {
        assert!(container_in_project("z3-regtest-zebra-1", "z3-regtest"));
        assert!(container_in_project("z3-regtest-zallet-1", "z3-regtest"));
        assert!(container_in_project("z3-regtest-zaino-1", "z3-regtest"));
        assert!(container_in_project(
            "z3-regtest-rpc-router-1",
            "z3-regtest"
        ));
    }

    #[test]
    fn container_in_project_matches_derived_env_id_project_names() {
        // The docker-stats sampler is scoped by `self.config.compose_project`
        // (see `spawn_resource_sampling`), which for a regtest run is now
        // `compose_project_for_env(env_id)` (`z3-sim-<env_id>`), not the old
        // literal `z3-regtest` — this function's prefix-match logic must work
        // identically for that generated name.
        let project = compose_project_for_env("a1b2c3d4");
        assert!(container_in_project("z3-sim-a1b2c3d4-zebra-1", &project));
        assert!(container_in_project("z3-sim-a1b2c3d4-zallet-1", &project));
        assert!(!container_in_project("z3-sim-00000000-zebra-1", &project));
    }

    #[test]
    fn container_in_project_excludes_other_networks_and_apps() {
        // Other Z3 networks must not be captured by the regtest sampler.
        assert!(!container_in_project("z3-mainnet-zebra-1", "z3-regtest"));
        assert!(!container_in_project("z3-testnet-zebra-1", "z3-regtest"));
        // Unrelated host containers are ignored.
        assert!(!container_in_project("some-other-app-1", "z3-regtest"));
        assert!(!container_in_project("postgres", "z3-regtest"));
    }

    #[test]
    fn container_in_project_requires_the_hyphen_separator() {
        // The trailing hyphen prevents a longer sibling project name from being
        // captured by a shorter one, and requires the literal `<project>-` boundary.
        assert!(!container_in_project("z3-regtestnet-zebra-1", "z3-regtest"));
        assert!(!container_in_project("z3regtest-zebra-1", "z3-regtest"));
        // The exact project name without the service suffix is not a container.
        assert!(!container_in_project("z3-regtest", "z3-regtest"));
    }

    #[test]
    fn parse_cpu_100_percent() {
        let v = serde_json::json!({"CPUPerc": "100.00%"});
        assert_eq!(parse_cpu_percent(&v), Some(100.0));
    }

    #[test]
    fn parse_cpu_fractional_below_one() {
        let v = serde_json::json!({"CPUPerc": "0.01%"});
        let got = parse_cpu_percent(&v).unwrap();
        assert!((got - 0.01).abs() < 1e-9, "expected 0.01, got {got}");
    }

    #[test]
    fn parse_cpu_missing_field_returns_none() {
        let v = serde_json::json!({});
        assert_eq!(parse_cpu_percent(&v), None);
    }

    #[test]
    fn parse_cpu_malformed_string_returns_none() {
        let v = serde_json::json!({"CPUPerc": "N/A%"});
        assert_eq!(parse_cpu_percent(&v), None);
    }

    // ── parse_mem_mb ─────────────────────────────────────────────────────────

    #[test]
    fn parse_mem_bytes_unit() {
        // 1,048,576 B = 1.0 MiB
        let v = serde_json::json!({"MemUsage": "1048576B / 16GiB"});
        let got = parse_mem_mb(&v).unwrap();
        assert!((got - 1.0).abs() < 1e-6, "expected 1.0 MB, got {got}");
    }

    #[test]
    fn parse_mem_unrecognized_suffix_returns_none() {
        let v = serde_json::json!({"MemUsage": "512TB / 1PB"});
        assert_eq!(parse_mem_mb(&v), None);
    }

    // ── health_check with mock HTTP server ────────────────────────────────────

    #[tokio::test]
    async fn health_check_returns_true_for_regtest_chain() {
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // Zebra regtest reports "test" (regtest is a Testnet variant in Zebra internals).
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"chain": "test", "blocks": 1, "headers": 1},
                "error": null,
                "id": 1
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        assert!(health_check(&client, &server.uri(), None).await);
    }

    #[tokio::test]
    async fn health_check_returns_true_for_test_chain() {
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"chain": "test", "blocks": 2, "headers": 2},
                "error": null,
                "id": 1
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        assert!(health_check(&client, &server.uri(), None).await);
    }

    #[tokio::test]
    async fn health_check_returns_false_for_wrong_chain() {
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"chain": "mainnet", "blocks": 100},
                "error": null,
                "id": 1
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        assert!(!health_check(&client, &server.uri(), None).await);
    }

    #[tokio::test]
    async fn health_check_returns_false_when_result_is_null() {
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": null,
                "error": {"code": -1, "message": "not ready"},
                "id": 1
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        assert!(!health_check(&client, &server.uri(), None).await);
    }

    #[tokio::test]
    async fn health_check_returns_false_for_json_rpc_error_response() {
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": null,
                "error": {"code": -32601, "message": "Method not found"},
                "id": 1
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        assert!(!health_check(&client, &server.uri(), None).await);
    }

    #[tokio::test]
    async fn health_check_returns_false_for_http_500() {
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        assert!(!health_check(&client, &server.uri(), None).await);
    }

    #[tokio::test]
    async fn health_check_returns_false_for_connection_refused() {
        // Grab a free port, release it, then immediately connect — the OS won't
        // reuse ephemeral ports that quickly, so this reliably hits ECONNREFUSED.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let client = reqwest::Client::new();
        assert!(!health_check(&client, &format!("http://{addr}"), None).await);
    }

    // ── parse_compose_images ─────────────────────────────────────────────────

    /// Captured verbatim from `docker compose images --format json` against a
    /// real, live-brought-up regtest stack (project `z3-sim-9387ad5f`) — not
    /// hand-constructed, so this fixture matches Docker's actual field names
    /// and casing (`ID`/`ContainerName`/`Repository`/`Tag`) exactly.
    const REAL_COMPOSE_IMAGES_JSON: &str = r#"[{"ID":"sha256:28bd5fe8b56d1bd048e5babf5b10710ebe0bae67db86916198a6eec434943f8b","ContainerName":"z3-sim-9387ad5f-cookie-permissions-1","Repository":"alpine","Tag":"3","Platform":"linux/arm64/v8","Size":4193907,"Created":"2026-06-16T00:01:20.474100947Z","LastTagTime":"2026-07-30T14:34:03.729680089Z"},{"ID":"sha256:28bd5fe8b56d1bd048e5babf5b10710ebe0bae67db86916198a6eec434943f8b","ContainerName":"z3-sim-9387ad5f-zallet-permissions-1","Repository":"alpine","Tag":"3","Platform":"linux/arm64/v8","Size":4193907,"Created":"2026-06-16T00:01:20.474100947Z","LastTagTime":"2026-07-30T14:34:03.729680089Z"},{"ID":"sha256:5b8dcbe525fd86a3faa469a497a0770c7b0512476467ac4b495c201dd5cd4040","ContainerName":"z3-sim-9387ad5f-rpc-router-1","Repository":"z3-sim-9387ad5f-rpc-router","Tag":"latest","Platform":"linux/arm64","Size":11584117,"Created":"2026-09-04T13:04:27.893797676Z","LastTagTime":"2026-09-04T16:25:22.222079005Z"},{"ID":"sha256:a9059a7b8a32d294905d1dcb72987d9dfd2443a39f3453d92a9e5b46aa523f9f","ContainerName":"z3-sim-9387ad5f-zallet-1","Repository":"z3sim/zallet","Tag":"v0.1.0-beta.2","Platform":"linux/amd64","Size":175070006,"Created":"2026-07-31T13:27:25.55376018Z","LastTagTime":"2026-07-31T13:27:29.277409001Z"},{"ID":"sha256:3ed6cbb5ed85d6a610ec5cf80cffb4a14a6f5b2517abad754a296cad95605bb0","ContainerName":"z3-sim-9387ad5f-zaino-1","Repository":"zingodevops/zainod","Tag":"0.6.0-no-tls","Platform":"linux/amd64","Size":46126956,"Created":"2026-07-13T20:25:28.831733091Z","LastTagTime":"2026-07-30T14:34:49.078297388Z"},{"ID":"sha256:78a10b7f24b83a86e6223d97e857094a353454e3268f84a87bd987e7140a33bb","ContainerName":"z3-sim-9387ad5f-zebra-1","Repository":"zfnd/zebra","Tag":"6.0.0","Platform":"linux/amd64","Size":118207384,"Created":"2026-07-10T21:17:50Z","LastTagTime":"2026-08-06T15:04:54.156594668Z"}]"#;

    #[test]
    fn parse_compose_images_matches_docker_compose_images_output() {
        let images = parse_compose_images(REAL_COMPOSE_IMAGES_JSON, "z3-sim-9387ad5f").unwrap();
        assert_eq!(
            images,
            vec![
                ImageInfo {
                    service: "zebra".into(),
                    image: "zfnd/zebra:6.0.0".into(),
                    id: "sha256:78a10b7f24b83a86e6223d97e857094a353454e3268f84a87bd987e7140a33bb"
                        .into(),
                },
                ImageInfo {
                    service: "zaino".into(),
                    image: "zingodevops/zainod:0.6.0-no-tls".into(),
                    id: "sha256:3ed6cbb5ed85d6a610ec5cf80cffb4a14a6f5b2517abad754a296cad95605bb0"
                        .into(),
                },
                ImageInfo {
                    service: "zallet".into(),
                    image: "z3sim/zallet:v0.1.0-beta.2".into(),
                    id: "sha256:a9059a7b8a32d294905d1dcb72987d9dfd2443a39f3453d92a9e5b46aa523f9f"
                        .into(),
                },
                ImageInfo {
                    service: "rpc-router".into(),
                    image: "z3-sim-9387ad5f-rpc-router:latest".into(),
                    id: "sha256:5b8dcbe525fd86a3faa469a497a0770c7b0512476467ac4b495c201dd5cd4040"
                        .into(),
                },
            ]
        );
    }

    #[test]
    fn parse_compose_images_excludes_permission_helper_containers() {
        let images = parse_compose_images(REAL_COMPOSE_IMAGES_JSON, "z3-sim-9387ad5f").unwrap();
        assert!(images.iter().all(|i| i.service != "cookie-permissions"));
        assert!(images.iter().all(|i| i.service != "zallet-permissions"));
        assert_eq!(images.len(), 4, "expected exactly the 4 VERSION_SERVICES");
    }

    #[test]
    fn parse_compose_images_null_yields_empty_not_error() {
        let images = parse_compose_images("null", "z3-sim-9387ad5f").unwrap();
        assert!(images.is_empty());
    }

    #[test]
    fn parse_compose_images_only_matches_own_project_prefix() {
        // A container from a DIFFERENT project must never be attributed to
        // this one, even if its own suffix looks like a known service.
        let json = r#"[{"ID":"sha256:abc","ContainerName":"z3-sim-other0000-zebra-1","Repository":"zfnd/zebra","Tag":"6.0.0"}]"#;
        let images = parse_compose_images(json, "z3-sim-9387ad5f").unwrap();
        assert!(images.is_empty());
    }

    #[test]
    fn parse_compose_images_rejects_malformed_json() {
        let err = parse_compose_images("not json", "z3-sim-9387ad5f").unwrap_err();
        assert!(matches!(err, Z3Error::ComposeCommand { .. }));
    }
}
