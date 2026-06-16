use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio::time::{self, MissedTickBehavior};

use crate::data_model::MetricSample;
use crate::metrics::MetricsRecorder;

pub mod contract;

use contract::{ContractError, Z3Contract};

const SERVICES: &[&str] = &["zebra", "zallet", "zaino"];
const ENV_FILE: &str = ".env.regtest";

/// Default RPC Router credentials for the regtest stack (see z3-contract.yaml
/// `rpc_auth.credential_env_vars`; the compose defaults are `zebra` / `zebra`).
const DEFAULT_REGTEST_RPC_USER: &str = "zebra";
const DEFAULT_REGTEST_RPC_PASSWORD: &str = "zebra";

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
}

impl Z3Config {
    /// Regtest defaults. Ports/credentials match the values in `z3-contract.yaml`;
    /// prefer [`Z3Config::from_contract`] to derive them from the checked-out Z3
    /// contract rather than relying on these constants.
    pub fn for_run(run_id: &str, log_dir: PathBuf) -> Self {
        Self {
            compose_dir: PathBuf::from("external/z3"),
            rpc_url: "http://127.0.0.1:8181".into(),
            basic_auth: Some((
                DEFAULT_REGTEST_RPC_USER.into(),
                DEFAULT_REGTEST_RPC_PASSWORD.into(),
            )),
            compose_project: "z3-regtest".into(),
            log_dir,
            run_id: run_id.into(),
            health_check_timeout_secs: 60,
            resource_sample_interval_secs: 5,
        }
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

        let rpc_url = net.primary_rpc_url("127.0.0.1")?;
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
            health_check_timeout_secs: 60,
            resource_sample_interval_secs: 5,
        })
    }
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum Z3Error {
    ComposeDirNotFound(PathBuf),
    EnvFileNotFound(PathBuf),
    LogDirCreate(std::io::Error),
    ComposeCommand { args: String, stderr: String },
    HealthCheckTimeout { after_secs: u64 },
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
        self.check_preconditions()?;
        tokio::fs::create_dir_all(&self.config.log_dir)
            .await
            .map_err(Z3Error::LogDirCreate)?;
        self.run_compose(&["up", "-d"]).await?;
        self.wait_until_ready().await?;
        self.spawn_log_capture();
        self.spawn_resource_sampling();
        Ok(())
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
        if !self.config.compose_dir.exists() {
            return Err(Z3Error::ComposeDirNotFound(self.config.compose_dir.clone()));
        }
        let env = self.config.compose_dir.join(ENV_FILE);
        if !env.exists() {
            return Err(Z3Error::EnvFileNotFound(env));
        }
        Ok(())
    }

    async fn run_compose(&self, args: &[&str]) -> Result<(), Z3Error> {
        let output = Command::new("docker")
            .arg("compose")
            .arg("--env-file")
            .arg(ENV_FILE)
            .args(args)
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
                return Ok(());
            }
            time::sleep(Duration::from_millis(500)).await;
        }
    }

    fn spawn_log_capture(&mut self) {
        for &service in SERVICES {
            let log_path = self.config.log_dir.join(format!("{}.log", service));
            let compose_dir = self.config.compose_dir.clone();
            let svc = service.to_string();
            self.background_tasks.push(tokio::spawn(async move {
                capture_logs(&compose_dir, &svc, &log_path).await;
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

    json.pointer("/result/chain")
        .and_then(|v| v.as_str())
        .map(|chain| chain == "regtest")
        .unwrap_or(false)
}

async fn capture_logs(compose_dir: &Path, service: &str, log_path: &Path) {
    let Ok(file) = tokio::fs::File::create(log_path).await else {
        return;
    };
    let mut writer = tokio::io::BufWriter::new(file);

    let Ok(mut child) = Command::new("docker")
        .arg("compose")
        .arg("--env-file")
        .arg(ENV_FILE)
        .arg("logs")
        .arg("--follow")
        .arg("--no-log-prefix")
        .arg(service)
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

async fn sample_resources(
    run_id: String,
    compose_project: String,
    interval_secs: u64,
    metrics: Option<Arc<dyn MetricsRecorder>>,
) {
    let Some(m) = metrics else { return };
    let mut ticker = time::interval(Duration::from_secs(interval_secs));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    // Compose v2 names containers `<project>-<service>-<n>`; we use this prefix
    // to scope `docker stats` (which samples all host containers) to our stack.
    let name_prefix = format!("{compose_project}-");

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
            if !name.starts_with(&name_prefix) {
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
        let config = Z3Config::for_run("test-run", PathBuf::from("/tmp/z3-test-logs"));
        let stack = Z3Stack::new(config, None);
        assert!(matches!(
            stack.check_preconditions(),
            Err(Z3Error::ComposeDirNotFound(_))
        ));
    }

    // ── Z3Config::for_run defaults ────────────────────────────────────────────

    #[test]
    fn z3config_for_run_sets_correct_defaults() {
        let cfg = Z3Config::for_run("run-42", PathBuf::from("/tmp/logs"));
        assert_eq!(cfg.compose_dir, PathBuf::from("external/z3"));
        assert_eq!(cfg.rpc_url, "http://127.0.0.1:8181");
        assert_eq!(
            cfg.basic_auth,
            Some(("zebra".to_string(), "zebra".to_string()))
        );
        assert_eq!(cfg.compose_project, "z3-regtest");
        assert_eq!(cfg.health_check_timeout_secs, 60);
        assert_eq!(cfg.resource_sample_interval_secs, 5);
        assert_eq!(cfg.run_id, "run-42");
        assert_eq!(cfg.log_dir, PathBuf::from("/tmp/logs"));
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
            health_check_timeout_secs: 60,
            resource_sample_interval_secs: 5,
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
            health_check_timeout_secs: 60,
            resource_sample_interval_secs: 5,
        };
        let stack = Z3Stack::new(config, None);
        assert!(stack.check_preconditions().is_ok());
    }

    // ── parse_cpu_percent ────────────────────────────────────────────────────

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
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"chain": "regtest", "blocks": 1, "headers": 1},
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
}
