FROM fedora:35

ENV PATH="/root/.cargo/bin:${PATH}"

RUN dnf install -y clang git openssl-devel

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y
RUN rustup install nightly
RUN rustup default nightly
RUN rustup component add rust-src
RUN rustup component add llvm-tools-preview

RUN cargo install cargo-audit grcov
