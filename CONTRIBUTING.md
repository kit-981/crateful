# Contributing

## License

All contributions must be licensed under [GPL version 3](https://www.gnu.org/licenses/gpl-3.0.txt)
or later.

## Lints

Linting is provided by [Clippy](https://github.com/rust-lang/rust-clippy). The project provides a
simple configuration in `clippy.toml`. Linting is automated by the `script/lint.sh` script.

## Tests

### Unit Tests

Unit tests should be written according to this short [blog
post](https://www.artima.com/weblogs/viewpost.jsp?thread=126923).

Unit tests are not written within source code files. Unit tests can be found in files called
`tests.rs` in the modules they belong to. This is done to reduce the complexity of generating useful
code coverage information.

### Integration Tests

Integration tests are all tests that are not unit tests. Integration test suites are stored in
`tests/`. Currently, only a single suite exists called `integration`. Integration test suites are
permitted to fail when run in parallel. However, integration tests within a suite are written to run
in parallel.

The integration tests for `crateful` are self-contained. To exercise components that use
[reqwest](https://docs.rs/reqwest/latest/reqwest/) and [git2](https://docs.rs/git2/latest/git2/),
the integration tests start their own web server and host simulated registries.

### Coverage

Test coverage for unit tests and integration tests can be generated with
[grcov](https://github.com/mozilla/grcov). This task is automated by the `scripts/coverage.sh`
script.

There is no desirable percentage for test coverage. Test coverage is a tool to ensure that tests
correctly exercise the components they are written to exercise and to provide suggestions for how
tests may be improved.
