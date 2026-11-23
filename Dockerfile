FROM rust:1.94-slim-bookworm AS builder

# Install openssl and protobuf compiler
RUN apt-get update && apt-get install -y pkg-config libssl-dev ca-certificates protobuf-compiler && rm -rf /var/lib/apt/lists/*

WORKDIR /app

ARG CARGO_FEATURES=

# Self-contained: buildable from this repo alone (docker build .), with only
# the default features. The optional "plugin" feature needs out-of-tree
# sources (see README) that this build context doesn't include.
RUN --mount=type=bind,source=src,target=src \
    --mount=type=bind,source=Cargo.toml,target=Cargo.toml \
    --mount=type=bind,source=Cargo.lock,target=Cargo.lock \
    --mount=type=bind,source=build.rs,target=build.rs \
    --mount=type=bind,source=migrations,target=migrations \
    --mount=type=cache,target=/app/target/ \
    --mount=type=cache,target=/usr/local/cargo/registry/ \
    cargo build --locked --release ${CARGO_FEATURES:+--features "$CARGO_FEATURES"} && \
    cp /app/target/release/transit-feed-processor /bin/app

# Runtime stage
FROM debian:bookworm-slim AS final

# Install CA certificates for SSL verification
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /bin/app /bin/app

ENV RUST_LOG="info"
CMD ["/bin/app"]
