# Local Configuration Overrides

This directory holds local environment-specific configuration files.

**These files are gitignored and must never be committed.**

## Purpose

Use this directory for:

- local port assignments for Z3 component processes,
- local paths to Zebra, Zaino, and Zallet binaries,
- local regtest chain data directories,
- any developer-specific settings that should not apply to other team members.

## Usage

The simulator does not currently read any file from this directory automatically —
it is reserved for developer-specific values (local paths, ports, chain data
locations) to reference manually or copy into a scenario override when needed.
