FROM ubuntu:24.04@sha256:224a1869083a311ef3f13648a154ba79832fbef6364d31493642ca03082da254

ARG RUSTUP_VERSION=1.28.2
ARG GO_VERSION=1.26.1
ARG GOLANGCI_LINT_VERSION=2.6.1
ARG NODE_VERSION=20.9.0

ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:/usr/local/go/bin:$PATH

# This set of packages is adapted from ./script/bootstrap in warp-internal and warp-server.
# It doesn't include everything (for example, wgslfmt), but is enough to build and test Warp.
# The verified tool archives below support amd64 and arm64.

# Install system-level packages. These are needed to build on Linux, and/or to run integration tests.
# APT verifies packages and repository metadata with Ubuntu's archive signatures.
RUN set -eux; \
  apt-get update; \
  apt-get install -y --no-install-recommends \
    pkg-config git build-essential libclang-dev curl unzip zsh fish vim locales sudo gnupg2 \
    protobuf-compiler libprotobuf-dev \
    fontconfig zlib1g libasound2-dev \
    libx11-6 libxcb1 libxi6 libxcursor1 libxkbcommon-x11-0 \
    libwayland-client0 libwayland-egl1 \
    mesa-vulkan-drivers libegl1 \
    libsqlite3-dev libpq-dev libmysqlclient-dev \
    ca-certificates \
    xvfb; \
  rm -rf /var/lib/apt/lists/*

# Install the pinned rustup-init binary after verifying the official archive checksum.
# --default-toolchain none avoids an implicit download before rust-toolchain.toml is applied.
RUN set -eux; \
  dpkg_arch="$(dpkg --print-architecture)"; \
  case "$dpkg_arch" in \
    amd64) rust_target="x86_64-unknown-linux-gnu"; rustup_sha256="20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c" ;; \
    arm64) rust_target="aarch64-unknown-linux-gnu"; rustup_sha256="e3853c5a252fca15252d07cb23a1bdd9377a8c6f3efa01531109281ae47f841c" ;; \
    *) echo "Unsupported architecture for rustup: $dpkg_arch" >&2; exit 1 ;; \
  esac; \
  rustup_init=/tmp/rustup-init; \
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${rust_target}/rustup-init" \
    --output "$rustup_init"; \
  echo "$rustup_sha256  $rustup_init" | sha256sum --check -; \
  chmod 0755 "$rustup_init"; \
  "$rustup_init" -y --no-modify-path --profile minimal --default-toolchain none; \
  rm "$rustup_init"; \
  chmod -R a+w "$RUSTUP_HOME" "$CARGO_HOME"; \
  rustup --version

# Install Go from its versioned archive, then install checksum-verified Go modules at exact versions.
RUN set -eux; \
  dpkg_arch="$(dpkg --print-architecture)"; \
  case "$dpkg_arch" in \
    amd64) go_arch="amd64"; go_sha256="031f088e5d955bab8657ede27ad4e3bc5b7c1ba281f05f245bcc304f327c987a" ;; \
    arm64) go_arch="arm64"; go_sha256="a290581cfe4fe28ddd737dde3095f3dbeb7f2e4065cab4eae44dfc53b760c2f7" ;; \
    *) echo "Unsupported architecture for Go: $dpkg_arch" >&2; exit 1 ;; \
  esac; \
  go_archive="/tmp/go${GO_VERSION}.linux-${go_arch}.tar.gz"; \
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    "https://go.dev/dl/go${GO_VERSION}.linux-${go_arch}.tar.gz" \
    --output "$go_archive"; \
  echo "$go_sha256  $go_archive" | sha256sum --check -; \
  tar -C /usr/local -xzf "$go_archive"; \
  rm "$go_archive"; \
  go version; \
  go env -w "GOPRIVATE=github.com/warpdotdev/*"; \
  CGO_ENABLED=0 go install -tags='no_clickhouse no_libsql no_mssql no_mysql no_sqlite3 no_vertica no_ydb' github.com/pressly/goose/v3/cmd/goose@v3.24.1; \
  echo "Installed goose"; \
  go install github.com/vektra/mockery/v2@v2.53.5; \
  echo "Installed mockery"; \
  go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.36.6; \
  echo "Installed protoc-gen-go"; \
  go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.2.0; \
  echo "Installed protoc-gen-go-grpc"

# Install golangci-lint from a versioned release archive instead of executing a remote installer.
RUN set -eux; \
  dpkg_arch="$(dpkg --print-architecture)"; \
  case "$dpkg_arch" in \
    amd64) golangci_arch="amd64"; golangci_sha256="c22e188e46aff9b140588abe6828ba271b600ae82b2d6a4f452196a639c17ec0" ;; \
    arm64) golangci_arch="arm64"; golangci_sha256="1c22b899f2dd84f9638e0e0352a319a2867b0bb082c5323ad50d8713b65bb793" ;; \
    *) echo "Unsupported architecture for golangci-lint: $dpkg_arch" >&2; exit 1 ;; \
  esac; \
  golangci_archive="/tmp/golangci-lint-${GOLANGCI_LINT_VERSION}-linux-${golangci_arch}.tar.gz"; \
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    "https://github.com/golangci/golangci-lint/releases/download/v${GOLANGCI_LINT_VERSION}/golangci-lint-${GOLANGCI_LINT_VERSION}-linux-${golangci_arch}.tar.gz" \
    --output "$golangci_archive"; \
  echo "$golangci_sha256  $golangci_archive" | sha256sum --check -; \
  tar -C /tmp -xzf "$golangci_archive"; \
  install -m 0755 "/tmp/golangci-lint-${GOLANGCI_LINT_VERSION}-linux-${golangci_arch}/golangci-lint" /usr/local/go/bin/golangci-lint; \
  rm -rf "$golangci_archive" "/tmp/golangci-lint-${GOLANGCI_LINT_VERSION}-linux-${golangci_arch}"; \
  golangci-lint --version

# Node 20.9.0 matches CI and still bundles Corepack, avoiding an additional npm installer download.
RUN set -eux; \
  dpkg_arch="$(dpkg --print-architecture)"; \
  case "$dpkg_arch" in \
    amd64) node_arch="x64"; node_sha256="9033989810bf86220ae46b1381bdcdc6c83a0294869ba2ad39e1061f1e69217a" ;; \
    arm64) node_arch="arm64"; node_sha256="ced3ecece4b7c3a664bca3d9e34a0e3b9a31078525283a6fdb7ea2de8ca5683b" ;; \
    *) echo "Unsupported architecture for Node.js: $dpkg_arch" >&2; exit 1 ;; \
  esac; \
  node_archive="/tmp/node-v${NODE_VERSION}-linux-${node_arch}.tar.xz"; \
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location --compressed \
    "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-${node_arch}.tar.xz" \
    --output "$node_archive"; \
  echo "$node_sha256  $node_archive" | sha256sum --check -; \
  tar -xJf "$node_archive" -C /usr/local --strip-components=1 --no-same-owner; \
  rm "$node_archive"; \
  ln -s /usr/local/bin/node /usr/local/bin/nodejs; \
  node --version; \
  npm --version; \
  corepack enable

# Ensure the exact Rust toolchain from the repository is installed.
COPY ./rust-toolchain.toml ./setup/rust-toolchain.toml
RUN cd ./setup && rustup toolchain install

# Install all dependencies needed to build, run, and test Warp. Package-manager downloads
# are versioned and verified by their native APT, Cargo, Go, or npm integrity mechanisms.
COPY ./script ./setup/script
RUN cd ./setup && INSTALL_GCLOUD_CLI=false ./script/linux/install_test_deps
