#!/bin/sh
RUSTFLAGS="-Ctarget-cpu=native" cargo +nightly build --release -Zbuild-std --target $1
