#!/bin/sh

cargo fmt --check &&
cargo clippy --all --tests
