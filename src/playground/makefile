check:
	cargo clean
	RUSTC_ICE=0 cargo run -Z bindeps -j 1 --manifest-path ../cargo/Cargo.toml -- check --old-artifacts=delete

run: check
	cargo run
