# Z3 Exchange Simulator — development command surface.
# Run `make help` to list available targets.

CARGO  := cargo
BINARY := z3sim

.DEFAULT_GOAL := help
.PHONY: help setup clone-z3 build build-release test fmt fmt-check lint \
        generate-fixtures scenario-dry-run clean

help: ## Show this help message
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		sort | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-22s\033[0m%s\n", $$1, $$2}'

setup: ## Check local development dependencies
	@command -v cargo >/dev/null 2>&1 || \
		{ echo "Error: cargo not found. Install Rust via https://rustup.rs"; exit 1; }
	@echo "cargo:  $$(cargo --version)"
	@echo "rustc:  $$(rustc --version)"

clone-z3: ## Clone pinned Z3 component repositories (Zebra, Zaino, Zallet)
	@bash scripts/dev/clone-z3.sh

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

generate-fixtures: ## Generate synthetic fixture data for tests
	@echo "TODO: implement synthetic fixture generator (Week 2)"

scenario-dry-run: ## Validate a scenario config without issuing RPC calls
	@echo "TODO: implement scenario dry-run (Week 2)"

clean: ## Remove build artifacts and generated experiment run outputs
	$(CARGO) clean
	@rm -rf experiments/runs/*/
	@echo "Build artifacts and experiment outputs removed."
