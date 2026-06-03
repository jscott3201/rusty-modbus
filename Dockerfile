# syntax=docker/dockerfile:1.7

ARG RUST_VERSION=1.95.0
ARG ALPINE_VERSION=3.22
ARG DISTROLESS_IMAGE=gcr.io/distroless/static-debian12:nonroot

FROM docker.io/library/rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS build
WORKDIR /app

RUN apk add --no-cache clang git lld musl-dev pkgconfig

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY benchmarks ./benchmarks

RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/usr/local/cargo/git/db \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --locked --release -p rusty-modbus-cli && \
    cp target/release/modbus /usr/local/bin/modbus

FROM build AS bench-build
RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/usr/local/cargo/git/db \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --locked --release -p rusty-modbus-benchmarks --bin stress-test && \
    cp target/release/stress-test /usr/local/bin/stress-test

FROM docker.io/library/alpine:${ALPINE_VERSION} AS runtime
ARG UID=10001

RUN apk add --no-cache ca-certificates && \
    adduser -D -H -h /nonexistent -s /sbin/nologin -u "${UID}" modbus

COPY --from=build /usr/local/bin/modbus /usr/local/bin/modbus

USER modbus
EXPOSE 5502
ENTRYPOINT ["/usr/local/bin/modbus"]
CMD ["--unit-id", "1", "server", "--listen", "0.0.0.0:5502"]

FROM docker.io/library/alpine:${ALPINE_VERSION} AS benchmark
ARG UID=10001

RUN apk add --no-cache ca-certificates && \
    adduser -D -H -h /nonexistent -s /sbin/nologin -u "${UID}" modbus

COPY --from=build /usr/local/bin/modbus /usr/local/bin/modbus
COPY --from=bench-build /usr/local/bin/stress-test /usr/local/bin/stress-test

USER modbus
ENTRYPOINT ["/usr/local/bin/stress-test"]
CMD ["--duration", "10", "--clients", "1", "--in-flight", "8", "--operation", "mixed", "--json"]

FROM ${DISTROLESS_IMAGE} AS distroless

COPY --from=build /usr/local/bin/modbus /usr/local/bin/modbus

USER 65532:65532
EXPOSE 5502
ENTRYPOINT ["/usr/local/bin/modbus"]
CMD ["--unit-id", "1", "server", "--listen", "0.0.0.0:5502"]

FROM ${DISTROLESS_IMAGE} AS benchmark-distroless

COPY --from=build /usr/local/bin/modbus /usr/local/bin/modbus
COPY --from=bench-build /usr/local/bin/stress-test /usr/local/bin/stress-test

USER 65532:65532
ENTRYPOINT ["/usr/local/bin/stress-test"]
CMD ["--duration", "10", "--clients", "1", "--in-flight", "8", "--operation", "mixed", "--json"]
