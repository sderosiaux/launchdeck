#!/usr/bin/env bash
set -euo pipefail

cargo fmt --check
cargo check --all-targets --all-features --message-format=short
cargo test --all-targets --all-features --message-format=short
cargo clippy --all-targets --all-features -- \
  -D warnings \
  -D clippy::dbg_macro \
  -D clippy::todo \
  -D clippy::unimplemented \
  -D clippy::unwrap_used \
  -D clippy::expect_used
