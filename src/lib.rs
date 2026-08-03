//! Z3 Exchange Simulator library.
//!
//! Stress-tests the Z3 Zcash stack (Zebra, Zaino, Zallet) under
//! exchange-scale synthetic load. All testing runs in regtest mode
//! with synthetically generated accounts and transactions.

pub mod cli;
pub mod data_model;
pub mod metrics;
pub mod report;
pub mod rpc;
pub mod scenarios;
pub mod synthetic;
pub mod z3;
