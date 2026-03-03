.PHONY: run build test fmt lint help

run:
	cargo run

build:
	cargo build

test:
	cargo test

fmt:
	cargo fmt

lint:
	cargo clippy --all-targets --all-features

help:
	cargo run -- --help
