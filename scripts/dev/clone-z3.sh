#!/usr/bin/env bash
# Clone Z3 component repositories at their pinned commits.
#
# Reads component definitions from z3-commits.lock at the repository root.
# Clones into external/ (gitignored).
#
# Usage:
#   bash scripts/dev/clone-z3.sh
#   make clone-z3

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXTERNAL_DIR="${REPO_ROOT}/external"
LOCK_FILE="${REPO_ROOT}/z3-commits.lock"

echo "Z3 Exchange Simulator — cloning Z3 component repositories"
echo "Lock file: ${LOCK_FILE}"
echo ""

if [[ ! -f "${LOCK_FILE}" ]]; then
  echo "Error: ${LOCK_FILE} not found." >&2
  exit 1
fi

mkdir -p "${EXTERNAL_DIR}"

# TODO: parse z3-commits.lock to extract repo URLs and commit hashes.
#       Implement this once commit hashes are confirmed at kickoff.

echo "TODO: clone Zebra"
echo "  repo:   https://github.com/ZcashFoundation/zebra"
echo "  commit: TBD — update z3-commits.lock after kickoff"
echo ""

echo "TODO: clone Zaino"
echo "  repo:   https://github.com/zingolabs/zaino"
echo "  commit: TBD — update z3-commits.lock after kickoff"
echo ""

echo "TODO: clone Zallet"
echo "  repo:   TBD — confirm repository URL before proceeding"
echo "  commit: TBD"
echo ""

echo "Skipping actual clones until commit hashes are pinned in z3-commits.lock."
echo "Re-run this script after the kickoff call."
