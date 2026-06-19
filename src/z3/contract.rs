//! Loader for `z3-contract.yaml` — Z3's canonical machine-readable inventory of
//! networks, ports, service DNS names, auth modes, and healthchecks.
//!
//! The simulator consumes this contract instead of hardcoding ports and service
//! names, so it stays correct as Z3 evolves. Only the fields the simulator needs
//! are modelled; serde ignores the rest (versioning, env-var schema, etc.).
//!
//! See the Z3 repo's `docs/contract.md` and `z3-contract.schema.json`.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

/// Filename of the contract within the cloned Z3 repository.
pub const CONTRACT_FILENAME: &str = "z3-contract.yaml";

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ContractError {
    Io(std::io::Error),
    Parse(String),
    MissingNetwork(String),
    MissingPort { network: String, port: String },
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContractError::Io(e) => write!(f, "failed to read z3-contract.yaml: {e}"),
            ContractError::Parse(e) => write!(f, "failed to parse z3-contract.yaml: {e}"),
            ContractError::MissingNetwork(n) => {
                write!(f, "network {n:?} not found in z3-contract.yaml")
            }
            ContractError::MissingPort { network, port } => {
                write!(f, "port {port:?} not defined for network {network:?}")
            }
        }
    }
}

impl std::error::Error for ContractError {}

// ── Contract model ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Z3Contract {
    pub contract_version: String,
    #[serde(default)]
    pub service_dns: HashMap<String, ServiceDns>,
    #[serde(default)]
    pub networks: HashMap<String, NetworkSpec>,
    #[serde(default)]
    pub healthchecks: HashMap<String, Healthcheck>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServiceDns {
    pub dns: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkSpec {
    pub z3_network: String,
    pub compose_project: String,
    pub external_network: String,
    pub rpc_auth: RpcAuth,
    #[serde(default)]
    pub ports: HashMap<String, PortSpec>,
    #[serde(default)]
    pub volumes: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RpcAuth {
    /// `cookie` (mainnet/testnet) or `username_password` (regtest).
    pub mode: String,
    #[serde(default)]
    pub credential_env_vars: Option<CredentialEnvVars>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CredentialEnvVars {
    pub user: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PortSpec {
    pub container: u16,
    /// Absent for ports that are not published to the host (e.g. regtest p2p).
    #[serde(default)]
    pub host: Option<u16>,
    /// Present only for ports that appear under a Compose profile (e.g. monitoring).
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Healthcheck {
    pub transport: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub liveness: Option<String>,
    #[serde(default)]
    pub readiness: Option<String>,
}

// ── Loading and accessors ─────────────────────────────────────────────────────

impl Z3Contract {
    pub fn from_yaml_str(s: &str) -> Result<Self, ContractError> {
        serde_yaml::from_str(s).map_err(|e| ContractError::Parse(e.to_string()))
    }

    /// Load the contract from a path (typically `external/z3/z3-contract.yaml`).
    pub fn from_path(path: &Path) -> Result<Self, ContractError> {
        let raw = std::fs::read_to_string(path).map_err(ContractError::Io)?;
        Self::from_yaml_str(&raw)
    }

    /// Load the contract from the Z3 compose directory.
    pub fn from_compose_dir(compose_dir: &Path) -> Result<Self, ContractError> {
        Self::from_path(&compose_dir.join(CONTRACT_FILENAME))
    }

    pub fn network(&self, name: &str) -> Result<&NetworkSpec, ContractError> {
        self.networks
            .get(name)
            .ok_or_else(|| ContractError::MissingNetwork(name.to_string()))
    }
}

impl NetworkSpec {
    /// Host-published port for a contract key (e.g. `"rpc_router"`, `"zaino_json_rpc"`).
    pub fn host_port(&self, key: &str) -> Option<u16> {
        self.ports.get(key).and_then(|p| p.host)
    }

    /// Like [`host_port`] but returns a descriptive error when the port is missing
    /// or not published to the host.
    pub fn require_host_port(&self, key: &str) -> Result<u16, ContractError> {
        self.host_port(key)
            .ok_or_else(|| ContractError::MissingPort {
                network: self.z3_network.clone(),
                port: key.to_string(),
            })
    }

    /// `http://<host>:<port>` for a contract port key, using the given host
    /// (typically `127.0.0.1`).
    pub fn http_url(&self, host: &str, port_key: &str) -> Result<String, ContractError> {
        Ok(format!(
            "http://{host}:{}",
            self.require_host_port(port_key)?
        ))
    }

    /// The RPC endpoint the simulator should drive: the regtest RPC Router if the
    /// contract defines one (regtest), otherwise Zebra's RPC port.
    pub fn primary_rpc_url(&self, host: &str) -> Result<String, ContractError> {
        if self.ports.contains_key("rpc_router") {
            self.http_url(host, "rpc_router")
        } else {
            self.http_url(host, "zebra_rpc")
        }
    }

    /// Whether this network authenticates with username/password (regtest) rather
    /// than a cookie file (mainnet/testnet).
    pub fn uses_username_password_auth(&self) -> bool {
        self.rpc_auth.mode == "username_password"
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // A representative slice of the real z3-contract.yaml (regtest + mainnet ports,
    // service DNS, healthchecks). Kept in sync with the upstream contract.
    const SAMPLE: &str = r#"
contract_version: "1.0.0"
service_dns:
  zebra:   {dns: zebra}
  zaino:   {dns: zaino}
  zallet:  {dns: zallet}
cookie_path: "/var/run/auth/.cookie"
networks:
  mainnet:
    z3_network: "Mainnet"
    compose_project: "z3-mainnet"
    external_network: "z3-mainnet"
    rpc_auth:
      mode: cookie
    ports:
      zebra_rpc:      {container: 8232,  host: 8232}
      zaino_json_rpc: {container: 8237,  host: 8237}
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
      zebra_rpc:      {container: 18232, host: 29232}
      zebra_p2p:      {container: 18233}
      zebra_health:   {container: 8080,  host: 28080}
      zaino_grpc:     {container: 8137,  host: 28137}
      zaino_json_rpc: {container: 8237,  host: 28237}
      zallet_rpc:     {container: 28232, host: 50232}
      rpc_router:     {container: 8181,  host: 8181}
      grafana:        {container: 3000,  host: 23000, profile: monitoring}
healthchecks:
  zebra:
    transport: http
    port: 8080
    liveness:  "/healthy"
    readiness: "/ready"
  zaino:
    transport: tcp
    port: 8137
  zallet:
    transport: none
"#;

    fn contract() -> Z3Contract {
        Z3Contract::from_yaml_str(SAMPLE).expect("sample parses")
    }

    #[test]
    fn parses_version_and_services() {
        let c = contract();
        assert_eq!(c.contract_version, "1.0.0");
        assert_eq!(c.service_dns["zebra"].dns, "zebra");
        assert_eq!(c.service_dns.len(), 3);
    }

    #[test]
    fn regtest_network_identifiers() {
        let net = contract().network("regtest").unwrap().clone();
        assert_eq!(net.z3_network, "Regtest");
        assert_eq!(net.compose_project, "z3-regtest");
        assert_eq!(net.external_network, "z3-regtest");
        assert!(net.uses_username_password_auth());
        let creds = net.rpc_auth.credential_env_vars.unwrap();
        assert_eq!(creds.user, "Z3_REGTEST_RPC_ROUTER_USER");
        assert_eq!(creds.password, "Z3_REGTEST_RPC_ROUTER_PASSWORD");
    }

    #[test]
    fn regtest_host_ports_match_contract() {
        let net = contract().network("regtest").unwrap().clone();
        assert_eq!(net.host_port("rpc_router"), Some(8181));
        assert_eq!(net.host_port("zebra_rpc"), Some(29232));
        assert_eq!(net.host_port("zaino_grpc"), Some(28137));
        assert_eq!(net.host_port("zaino_json_rpc"), Some(28237));
        assert_eq!(net.host_port("zallet_rpc"), Some(50232));
        assert_eq!(net.host_port("zebra_health"), Some(28080));
    }

    #[test]
    fn unpublished_port_has_no_host_mapping() {
        let net = contract().network("regtest").unwrap().clone();
        // p2p is not published on regtest.
        assert_eq!(net.host_port("zebra_p2p"), None);
        assert!(net.require_host_port("zebra_p2p").is_err());
    }

    #[test]
    fn primary_rpc_url_prefers_router_on_regtest() {
        let net = contract().network("regtest").unwrap().clone();
        assert_eq!(
            net.primary_rpc_url("127.0.0.1").unwrap(),
            "http://127.0.0.1:8181"
        );
    }

    #[test]
    fn primary_rpc_url_falls_back_to_zebra_without_router() {
        // mainnet has no rpc_router → falls back to zebra_rpc.
        let net = contract().network("mainnet").unwrap().clone();
        assert!(!net.uses_username_password_auth());
        assert_eq!(
            net.primary_rpc_url("127.0.0.1").unwrap(),
            "http://127.0.0.1:8232"
        );
    }

    #[test]
    fn zaino_json_rpc_url_built_from_contract() {
        let net = contract().network("regtest").unwrap().clone();
        assert_eq!(
            net.http_url("127.0.0.1", "zaino_json_rpc").unwrap(),
            "http://127.0.0.1:28237"
        );
    }

    #[test]
    fn healthchecks_parsed() {
        let c = contract();
        let zebra = &c.healthchecks["zebra"];
        assert_eq!(zebra.transport, "http");
        assert_eq!(zebra.port, Some(8080));
        assert_eq!(zebra.readiness.as_deref(), Some("/ready"));
        assert_eq!(c.healthchecks["zallet"].transport, "none");
    }

    #[test]
    fn missing_network_errors() {
        assert!(matches!(
            contract().network("does-not-exist"),
            Err(ContractError::MissingNetwork(_))
        ));
    }

    #[test]
    fn parses_the_full_upstream_contract_if_present() {
        // When run from a checkout that has vendored the contract, parse it too.
        // Skipped silently if the file isn't present (e.g. CI without external/z3).
        let path = std::path::Path::new("external/z3/z3-contract.yaml");
        if path.exists() {
            let c = Z3Contract::from_path(path).expect("upstream contract parses");
            let net = c.network("regtest").expect("regtest present");
            assert_eq!(net.host_port("rpc_router"), Some(8181));
        }
    }
}
