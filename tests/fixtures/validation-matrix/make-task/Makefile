syntax:
	bash -n scripts/*.sh
unit:
	bats tests/unit
integration:
	bats tests/integration
build:
	cargo build --workspace
smoke:
	./scripts/smoke.sh
