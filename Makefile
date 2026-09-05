# Z3 Exchange Simulator — development command surface.
# Run `make help` to list available targets.

CARGO  := cargo
BINARY := z3sim

.DEFAULT_GOAL := help
.PHONY: help setup clone-z3 bootstrap regtest-reset build build-release test fmt fmt-check lint \
        generate-fixtures scenario-dry-run validate-scenario clean

help: ## Show this help message
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		sort | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-22s\033[0m%s\n", $$1, $$2}'

setup: ## Check local development dependencies
	@command -v cargo >/dev/null 2>&1 || \
		{ echo "Error: cargo not found. Install Rust via https://rustup.rs"; exit 1; }
	@echo "cargo:  $$(cargo --version)"
	@echo "rustc:  $$(rustc --version)"

clone-z3: ## Clone the pinned Z3 Docker Compose stack (meta-repo) into external/z3
	@bash scripts/dev/clone-z3.sh

bootstrap: ## Check dependencies, apply the working override stack, and bring it up (run after clone-z3)
	@bash scripts/dev/bootstrap.sh

regtest-reset: ## Wipe and reinitialize the regtest chain/wallet (fixes funding failures from accumulated history)
	@bash scripts/dev/regtest-reset.sh --yes

build: ## Build the simulator binary (debug mode)
	$(CARGO) build

build-release: ## Build an optimized release binary
	$(CARGO) build --release

test: ## Run all tests
	$(CARGO) test

fmt: ## Format source code
	$(CARGO) fmt

fmt-check: ## Check formatting without modifying files (CI)
	$(CARGO) fmt -- --check

lint: ## Run clippy lints (warnings treated as errors)
	$(CARGO) clippy -- -D warnings

generate-fixtures: build ## Generate synthetic fixture data for tests
	$(CARGO) run -- generate-fixtures \
		--scenario configs/scenarios/smoke.yaml \
		--out experiments/fixtures

scenario-dry-run: build ## Validate and summarise smoke scenario without starting Z3
	$(CARGO) run -- run \
		--scenario configs/scenarios/smoke.yaml \
		--dry-run

validate-scenario: build ## Validate a scenario YAML file (usage: make validate-scenario SCENARIO=<path>)
	$(CARGO) run -- validate-scenario \
		$(or $(SCENARIO),configs/scenarios/smoke.yaml)

clean: ## Remove build artifacts and generated experiment run outputs
	$(CARGO) clean
	@rm -rf experiments/runs/*/
	@echo "Build artifacts and experiment outputs removed."
