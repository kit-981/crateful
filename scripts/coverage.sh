#!/bin/sh

# Clean previous runs.
rm -f *.profraw

export RUSTFLAGS="-Cinstrument-coverage"
export LLVM_PROFILE_FILE="%p-%m.profraw"

cargo +nightly build &&
RUST_BACKTRACE=1 cargo +nightly test -q --all &&
grcov . -s . --binary-path ./target/debug/ -t html --llvm --branch --ignore-not-existing -o ./target/debug/coverage/ --ignore '*tests.rs' --ignore "tests/*" &&


echo "Coverage information is avaiable at target/debug/coverage/index.html"
