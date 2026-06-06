//! Scenario runner and workload shaping.
//!
//! Parses scenario YAML configs, provisions synthetic accounts, and drives
//! the configured load profile (steady-state, ramp, burst, or mixed)
//! against the live Z3 stack via the RPC client.

pub mod exchange;
