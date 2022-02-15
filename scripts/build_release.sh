#!/bin/sh
cargo +nightly build --release -Zbuild-std --target $1
