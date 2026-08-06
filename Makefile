CG		?= cargo

.SILENT:
.PHONY: run build fmt clean rdme ci

# Install the prerequisites to fully use this file:
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
	$(CG) rdme


ci:
	$(CG) fmt --all --check
	$(CG) rdme --check
