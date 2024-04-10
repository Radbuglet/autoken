check:
	RUSTC_ICE=0 cargo run -Z bindeps --manifest-path ../cargo/Cargo.toml -- check --old-artifacts=delete

run: check
	cargo run
