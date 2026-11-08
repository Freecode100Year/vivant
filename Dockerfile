# VIVANT — container image.
#
#   docker build -t vivant .
#   docker run -d --name vivant --network host -v vivant-data:/data vivant
#
# Use host networking (or make sure Docker keeps client source addresses):
# behind Docker's userland proxy every user would appear to come from one
# address and share a single rate-limit bucket. See docs/deploy.md.
#
# Multi-arch images are cross-compiled on the build machine's own
# architecture (tonistiigi/xx), not emulated: compiling Rust under QEMU
# would take ages on arm/v7.

FROM --platform=$BUILDPLATFORM tonistiigi/xx:1.6.1 AS xx

FROM --platform=$BUILDPLATFORM rust:1-alpine AS build
COPY --from=xx / /
RUN apk add --no-cache clang lld
ARG TARGETPLATFORM
RUN xx-apk add --no-cache musl-dev gcc
WORKDIR /src
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY src ./src
ARG VERSION=dev
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=target-$TARGETPLATFORM \
    VIVANT_VERSION=${VERSION} xx-cargo build --release --locked --bin vivant \
 && xx-verify --static target/$(xx-cargo --print-target-triple)/release/vivant \
 && cp target/$(xx-cargo --print-target-triple)/release/vivant /vivant \
 && mkdir -p /out/data

# distroless/static: no shell, no package manager; runs as uid 65532.
FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=build /vivant /usr/local/bin/vivant
COPY --from=build --chown=65532:65532 /out/data /data
ENV VIVANT_ADDR=:2222 \
    VIVANT_HOST_KEY=/data/host_ed25519 \
    VIVANT_HEALTH_ADDR=127.0.0.1:2223
WORKDIR /data
VOLUME /data
EXPOSE 2222
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/vivant"]
