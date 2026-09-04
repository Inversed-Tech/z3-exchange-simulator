//! Per-checkout environment identity.
//!
//! Two simulator checkouts on the same host must never collide on Compose
//! project name, network, volumes, host ports, or subnet. This module derives
//! a stable identifier for the current checkout (cached at
//! `configs/local/env-id`) and the port/subnet values keyed off it, so Docker
//! and the simulator's own RPC client agree on one set of values.

use std::path::Path;

use rand::rngs::OsRng;
use rand::RngCore;

use crate::z3::Z3Error;

/// Resolve this checkout's environment id.
///
/// Stable per checkout by default: a cached value at `cache_path` is reused
/// across invocations so a reused-state workflow keeps talking to the same
/// Compose project. Pass `fresh = true` (the CLI's `--fresh-env` flag) to
/// discard any cached value and mint a new, disposable one.
pub fn resolve_env_id(cache_path: &Path, fresh: bool) -> Result<String, Z3Error> {
    if !fresh {
        if let Ok(existing) = std::fs::read_to_string(cache_path) {
            let trimmed = existing.trim();
            if is_valid_env_id(trimmed) {
                return Ok(trimmed.to_string());
            }
        }
    }
    let id = generate_env_id();
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent).map_err(Z3Error::EnvIdCacheIo)?;
    }
    std::fs::write(cache_path, &id).map_err(Z3Error::EnvIdCacheIo)?;
    Ok(id)
}

fn generate_env_id() -> String {
    let mut bytes = [0u8; 4];
    OsRng.fill_bytes(&mut bytes);
    hex_encode(&bytes)
}

fn hex_encode(bytes: &[u8; 4]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn is_valid_env_id(s: &str) -> bool {
    s.len() == 8
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Docker Compose project name for a given environment id, e.g. `z3-sim-a1b2c3d4`.
pub fn compose_project_for_env(env_id: &str) -> String {
    format!("z3-sim-{env_id}")
}

// ── Port derivation ──────────────────────────────────────────────────────────

/// Host ports the simulator and Docker must agree on, derived from an
/// `env_id`. Consumed by `z3::Z3Config::for_run`, which passes these as
/// process environment variables on every `docker compose` invocation it
/// makes (see `z3::compose_env_overrides`) — Compose's variable-interpolation
/// precedence puts the invoking process's environment above `--env-file`, so
/// this takes effect without ever rewriting `.env.regtest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortSet {
    pub zebra_rpc: u16,
    pub zebra_health: u16,
    pub zaino_grpc: u16,
    pub zaino_json_rpc: u16,
    pub zallet_rpc: u16,
    pub rpc_router: u16,
}

// Each of the six ports below covers a distinct `base..=base+9_999` range —
// one per host port `docker compose config` actually publishes for the
// regtest stack (confirmed against its resolved output, not just the ports
// referenced elsewhere in this codebase by name). Consecutive multiples of
// 10_000 starting at 1024: this is what actually guarantees the six ranges
// are pairwise disjoint (`base_ports_are_pairwise_disjoint` proves it by
// construction) — an earlier version of these constants "reused" pre-existing
// literal port defaults instead of spacing them out and silently overlapped
// two of the six ranges by ~9_000 values, which let two different
// environments collide on different port roles (e.g. one environment's
// `zebra_rpc` landing on another's `zaino_json_rpc`) despite each
// environment's own six ports never colliding with each other.
const BASE_RPC_ROUTER_PORT: u16 = 1_024;
const BASE_ZAINO_GRPC_PORT: u16 = 11_024;
const BASE_ZAINO_JSON_RPC_PORT: u16 = 21_024;
const BASE_ZEBRA_RPC_PORT: u16 = 31_024;
const BASE_ZEBRA_HEALTH_PORT: u16 = 41_024;
const BASE_ZALLET_RPC_PORT: u16 = 51_024;

/// Deterministically derive this environment's default host ports from its
/// `env_id`. Always in `1024..65535`, identical across repeated calls for the
/// same `env_id`.
pub fn derive_ports(env_id: &str) -> Result<PortSet, Z3Error> {
    let offset = env_id_offset(env_id)?;
    Ok(PortSet {
        zebra_rpc: apply_offset(BASE_ZEBRA_RPC_PORT, offset),
        zebra_health: apply_offset(BASE_ZEBRA_HEALTH_PORT, offset),
        zaino_grpc: apply_offset(BASE_ZAINO_GRPC_PORT, offset),
        zaino_json_rpc: apply_offset(BASE_ZAINO_JSON_RPC_PORT, offset),
        zallet_rpc: apply_offset(BASE_ZALLET_RPC_PORT, offset),
        rpc_router: apply_offset(BASE_RPC_ROUTER_PORT, offset),
    })
}

fn env_id_offset(env_id: &str) -> Result<u32, Z3Error> {
    u32::from_str_radix(env_id, 16)
        .map_err(|_| Z3Error::InvalidEnvId(env_id.to_string()))
        .map(|v| v % 10_000)
}

fn apply_offset(base: u16, offset: u32) -> u16 {
    // Bases + the widest offset (9_999) never approach u16::MAX, but clamp
    // defensively rather than let a future base constant overflow silently.
    (base as u32 + offset).min(65_535) as u16
}

// ── Subnet derivation ────────────────────────────────────────────────────────

/// A per-environment `/24` subnet plus the static address Zaino's public-bind
/// workaround needs within it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubnetAssignment {
    pub subnet: String,
    pub zaino_ip: String,
}

/// Deterministically derive one of 100 non-overlapping `/24` subnets in
/// `10.100.0.0/16`-`10.199.0.0/16` from `env_id` — chosen outside both
/// Docker's own default bridge range and common home-router ranges.
pub fn derive_subnet(env_id: &str) -> Result<SubnetAssignment, Z3Error> {
    let byte_hex = env_id
        .get(4..6)
        .ok_or_else(|| Z3Error::InvalidEnvId(env_id.to_string()))?;
    let byte =
        u8::from_str_radix(byte_hex, 16).map_err(|_| Z3Error::InvalidEnvId(env_id.to_string()))?;
    let octet = 100 + (byte % 100);
    Ok(SubnetAssignment {
        subnet: format!("10.{octet}.0.0/24"),
        zaino_ip: format!("10.{octet}.0.10"),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_env_id_persists_across_calls() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache_path = dir.path().join("env-id");

        let first = resolve_env_id(&cache_path, false).unwrap();
        let second = resolve_env_id(&cache_path, false).unwrap();
        assert_eq!(first, second, "stable calls must reuse the cached id");

        let fresh = resolve_env_id(&cache_path, true).unwrap();
        assert_ne!(first, fresh, "--fresh-env must mint a new id");
        let cached_after_fresh = std::fs::read_to_string(&cache_path).unwrap();
        assert_eq!(cached_after_fresh.trim(), fresh);
    }

    #[test]
    fn resolve_env_id_creates_parent_directories() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache_path = dir.path().join("nested").join("env-id");
        let id = resolve_env_id(&cache_path, false).unwrap();
        assert!(is_valid_env_id(&id));
    }

    #[test]
    fn resolve_env_id_regenerates_on_corrupt_cache() {
        let dir = tempfile::TempDir::new().unwrap();
        let cache_path = dir.path().join("env-id");
        std::fs::write(&cache_path, "not-hex!!").unwrap();
        let id = resolve_env_id(&cache_path, false).unwrap();
        assert!(is_valid_env_id(&id));
    }

    #[test]
    fn compose_project_for_env_is_deterministic_and_distinct() {
        assert_eq!(compose_project_for_env("a1b2c3d4"), "z3-sim-a1b2c3d4");
        assert_eq!(
            compose_project_for_env("a1b2c3d4"),
            compose_project_for_env("a1b2c3d4")
        );
        assert_ne!(
            compose_project_for_env("a1b2c3d4"),
            compose_project_for_env("00000000")
        );
    }

    #[test]
    fn derive_ports_is_deterministic_and_in_valid_range() {
        for env_id in ["00000000", "a1b2c3d4", "ffffffff", "00002710"] {
            let a = derive_ports(env_id).unwrap();
            let b = derive_ports(env_id).unwrap();
            assert_eq!(a, b, "must be deterministic for {env_id}");
            let ports = [
                a.zebra_rpc,
                a.zebra_health,
                a.zaino_grpc,
                a.zaino_json_rpc,
                a.zallet_rpc,
                a.rpc_router,
            ];
            for port in ports {
                assert!((1024..65535).contains(&port), "{port} out of range");
            }
            let mut sorted = ports.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(
                sorted.len(),
                ports.len(),
                "no two of this environment's own ports may collide with each other: {ports:?}"
            );
        }
    }

    #[test]
    fn derive_ports_differs_across_env_ids() {
        let a = derive_ports("00000001").unwrap();
        let b = derive_ports("00000002").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn derive_ports_rejects_invalid_env_id() {
        assert!(matches!(
            derive_ports("not-hex!"),
            Err(Z3Error::InvalidEnvId(_))
        ));
    }

    /// Proves the six base port ranges (`base..=base+9_999`) are pairwise
    /// disjoint by construction, not merely for the specific env-ids other
    /// tests happen to exercise. Range overlap between two DIFFERENT bases
    /// is what let two DIFFERENT environments collide on different port
    /// roles (e.g. one's `zebra_rpc` landing on another's
    /// `zaino_json_rpc`) even though `derive_ports_is_deterministic_and_in_valid_range`
    /// already proves no environment ever collides with ITSELF — a defect
    /// that check cannot catch, since it only ever inspects one env-id's
    /// own six ports at a time.
    #[test]
    fn base_ports_are_pairwise_disjoint() {
        let mut ranges: Vec<(u16, u32)> = [
            BASE_RPC_ROUTER_PORT,
            BASE_ZAINO_GRPC_PORT,
            BASE_ZAINO_JSON_RPC_PORT,
            BASE_ZEBRA_RPC_PORT,
            BASE_ZEBRA_HEALTH_PORT,
            BASE_ZALLET_RPC_PORT,
        ]
        .into_iter()
        .map(|base| (base, base as u32 + 9_999))
        .collect();
        ranges.sort_unstable_by_key(|(base, _)| *base);

        for pair in ranges.windows(2) {
            let (_, prev_end) = pair[0];
            let (next_base, _) = pair[1];
            assert!(
                next_base as u32 > prev_end,
                "base port ranges overlap: {:?} vs {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    /// Regression guard for the specific collision reported against an
    /// earlier version of the base constants: offset 0 (env-id "00000000")
    /// and offset 995 used to derive the same host port for two different
    /// port roles (`zebra_rpc` for the first, `zaino_json_rpc` for the
    /// second) because `BASE_ZAINO_JSON_RPC_PORT`'s range overlapped
    /// `BASE_ZEBRA_RPC_PORT`'s by exactly that offset delta.
    #[test]
    fn derive_ports_offsets_0_and_995_share_no_port() {
        let env_id_offset_0 = "00000000";
        let env_id_offset_995 = "000003e3"; // 0x3e3 = 995
        assert_eq!(env_id_offset(env_id_offset_995).unwrap(), 995);

        let a = derive_ports(env_id_offset_0).unwrap();
        let b = derive_ports(env_id_offset_995).unwrap();
        let a_ports = [
            a.zebra_rpc,
            a.zebra_health,
            a.zaino_grpc,
            a.zaino_json_rpc,
            a.zallet_rpc,
            a.rpc_router,
        ];
        let b_ports = [
            b.zebra_rpc,
            b.zebra_health,
            b.zaino_grpc,
            b.zaino_json_rpc,
            b.zallet_rpc,
            b.rpc_router,
        ];
        for pa in a_ports {
            assert!(
                !b_ports.contains(&pa),
                "port {pa} shared between offset-0 and offset-995 environments: {a_ports:?} vs {b_ports:?}"
            );
        }
    }

    #[test]
    fn derive_subnet_is_deterministic_and_in_reserved_range() {
        for env_id in ["00000000", "a1b2c3d4", "ffffffff"] {
            let a = derive_subnet(env_id).unwrap();
            let b = derive_subnet(env_id).unwrap();
            assert_eq!(a, b);
            assert!(a.subnet.starts_with("10."), "{}", a.subnet);
            assert!(a.subnet.ends_with(".0.0/24"), "{}", a.subnet);
            assert!(a.zaino_ip.ends_with(".0.10"), "{}", a.zaino_ip);
        }
    }

    #[test]
    fn derive_subnet_octet_within_expected_window() {
        // byte % 100 is 0..=99, offset by 100 -> 100..=199.
        for env_id in ["00000000", "0000ff00", "a1b2c3d4"] {
            let assignment = derive_subnet(env_id).unwrap();
            let octet: u16 = assignment
                .subnet
                .split('.')
                .nth(1)
                .unwrap()
                .parse()
                .unwrap();
            assert!((100..200).contains(&octet), "{octet} out of window");
        }
    }

    #[test]
    fn is_valid_env_id_rejects_malformed_input() {
        assert!(is_valid_env_id("a1b2c3d4"));
        assert!(!is_valid_env_id("A1B2C3D4")); // uppercase not accepted
        assert!(!is_valid_env_id("a1b2c3d")); // too short
        assert!(!is_valid_env_id("a1b2c3d4e")); // too long
        assert!(!is_valid_env_id("zzzzzzzz")); // non-hex
    }
}
