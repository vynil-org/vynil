ARG RUST_VERSION=1.99
ARG DEBIAN_VERSION=trixie
FROM --platform=${BUILDPLATFORM:-linux/amd64} rust:${RUST_VERSION}-slim-${DEBIAN_VERSION} AS builder
ARG BUILD_DEPS="binutils libssl-dev pkg-config git build-essential g++"
WORKDIR /usr/src/operator
COPY Cargo.lock .
# hadolint ignore=DL3008,DL3015
RUN mkdir -p operator/src common/src \
 && echo 'fn main() { println!("Dummy") }' > operator/src/main.rs \
 && touch common/src/lib.rs operator/src/lib.rs \
 && DEBIAN_FRONTEND=noninteractive apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get -y install ${BUILD_DEPS}
COPY operator/parent.toml ./Cargo.toml
COPY src src/
COPY common/Cargo.toml common/Cargo.toml
COPY operator/Cargo.toml operator/Cargo.toml
# Build the deps for caching purpose
RUN CARGO_NET_GIT_FETCH_WITH_CLI=true cargo build -r --bin operator
# Get our sources
COPY common common/
COPY operator operator/
# Update the crates entrypoints so cargo is aware these should rebuild
RUN touch common/src/lib.rs operator/src/lib.rs operator/src/main.rs \
 && cargo build -r --bin operator \
 && strip target/release/operator

# Finally create the target image with run-time dependencies installed
FROM debian:${DEBIAN_VERSION}-slim AS target
ARG DEB_PACKAGES="openssl"
# hadolint ignore=DL3027,DL3008
RUN DEBIAN_FRONTEND=noninteractive apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get -y upgrade \
 && DEBIAN_FRONTEND=noninteractive apt-get -y install --no-install-recommends ${DEB_PACKAGES} \
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/* \
 && mkdir -p /work \
 && chown nobody:nogroup /work
COPY --from=builder /usr/src/operator/target/release/operator /usr/bin/operator
COPY operator/templates /usr/lib/vynil/templates
USER nobody
WORKDIR /work
ENV CONTROLLER_BASE_DIR=/usr/lib/vynil
ENTRYPOINT ["operator"]
