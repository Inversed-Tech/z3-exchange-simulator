#!/usr/bin/env bash
# Clone the Z3 Docker Compose stack at the pinned commit.
#
# Reads the z3 commit from z3-commits.lock at the repository root.
# Clones into external/z3 (gitignored).
#
# Usage:
#   bash scripts/dev/clone-z3.sh
#   make clone-z3

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXTERNAL_DIR="${REPO_ROOT}/external"
LOCK_FILE="${REPO_ROOT}/z3-commits.lock"
Z3_REPO="https://github.com/ZcashFoundation/z3"
Z3_BRANCH="dev"
Z3_DIR="${EXTERNAL_DIR}/z3"

echo "Z3 Exchange Simulator — cloning Z3 stack"
echo "Lock file: ${LOCK_FILE}"
echo ""

if [[ ! -f "${LOCK_FILE}" ]]; then
  echo "Error: ${LOCK_FILE} not found." >&2
  exit 1
fi

mkdir -p "${EXTERNAL_DIR}"

# Extract z3 commit from lock file.
# Falls back to branch head if commit is TBD (pending Foundation confirmation).
Z3_COMMIT=$(grep -A2 '^z3:' "${LOCK_FILE}" | grep 'commit:' | awk '{print $2}' | tr -d '"')

if [[ -z "${Z3_COMMIT}" || "${Z3_COMMIT}" == "TBD" ]]; then
  echo "Warning: z3 commit is TBD in ${LOCK_FILE}."
  echo "Cloning latest dev branch. Update z3-commits.lock once commit is confirmed."
  echo ""
  if [[ -d "${Z3_DIR}/.git" ]]; then
    echo "Z3 already cloned at ${Z3_DIR} — skipping."
  else
    git clone --branch "${Z3_BRANCH}" "${Z3_REPO}" "${Z3_DIR}"
    echo "Cloned Z3 (dev HEAD) to ${Z3_DIR}"
  fi
else
  if [[ -d "${Z3_DIR}/.git" ]]; then
    echo "Z3 already cloned at ${Z3_DIR}."
    echo "Checking out pinned commit ${Z3_COMMIT}..."
    git -C "${Z3_DIR}" checkout "${Z3_COMMIT}"
  else
    git clone --branch "${Z3_BRANCH}" "${Z3_REPO}" "${Z3_DIR}"
    git -C "${Z3_DIR}" checkout "${Z3_COMMIT}"
    echo "Cloned Z3 at commit ${Z3_COMMIT} to ${Z3_DIR}"
  fi
fi

echo ""
echo "Next steps:"
echo "  cd ${Z3_DIR}"
echo "  ./scripts/regtest-init.sh            # one-time regtest setup"
echo "  docker compose --env-file .env.regtest up -d"
