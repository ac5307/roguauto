# Copyright (C) 2026 ac5307
# SPDX-License-Identifier: GPL-3.0-or-later

CG		?= cargo

.SILENT:
.PHONY: install run build fmt clean rdme ci

# The prerequisites to fully use this file:
# - Install the Rust toolchain.
# - Install `GNU make` or some other tooling that allows/has it.
# - Run `make install` in the project's root directory.
install:
	$(CG) install cargo-rdme
	$(CG) rdme install-rust-toolchain-for-intralinks


# Use `cargo run` instead if just checking for functionality.
# Otherwise, this is for responsiveness in the optimized build.
run: build
	./target/release/rogu


build:
	$(CG) build --release


fmt:
	$(CG) fmt


clean:
	$(CG) clean


rdme:
	$(CG) rdme --force


ci:
	$(CG) fmt --all --check
	$(CG) rdme --check
