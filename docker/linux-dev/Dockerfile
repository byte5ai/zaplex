FROM debian:sid-20260824@sha256:c1acdb109bacb5adf0f2078892ac177ee2e2ce6f88e96c5b743998b5489d36a9

ARG RUSTUP_VERSION=1.28.2

# Install utilities and libraries needed for development. APT verifies packages and repository
# metadata with Debian's archive signatures.
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        sudo \
        locales \
        zsh \
        curl \
        git openssh-client \
        nano vim \
        build-essential cmake pkg-config \
        python-is-python3 \
        libssl-dev libfreetype-dev libexpat1-dev \
        xserver-xorg libxcursor1 \
        mesa-vulkan-drivers mesa-utils \
        protobuf-compiler \
        ca-certificates; \
    rm -rf /var/lib/apt/lists/*

# The container is entered with docker exec, so the development account has no login password.
RUN set -eux; \
    useradd --create-home --user-group --shell /usr/bin/zsh --groups sudo --uid 1000 dev; \
    passwd --lock dev; \
    echo 'dev ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/dev; \
    chmod 0440 /etc/sudoers.d/dev; \
    install -d -o dev -g dev -m 0700 /run/user/dev; \
    install -d -o dev -g dev /home/dev/cargo-output /home/dev/setup

# Install the pinned rustup-init binary only after checking its official SHA-256 checksum.
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
    su - dev -c "$rustup_init -y --no-modify-path --profile minimal --default-toolchain none"; \
    rm "$rustup_init"

ENV CARGO_TARGET_DIR=/home/dev/cargo-output
ENV DISPLAY=host.docker.internal:0
ENV XDG_RUNTIME_DIR=/run/user/dev
ENV PATH=/home/dev/.cargo/bin:$PATH

# Generate the basic English UTF-8 locale.
RUN locale-gen en_US.UTF-8

# Install the exact Rust toolchain used to build Warp.
COPY rust-toolchain.toml /home/dev/setup/rust-toolchain.toml
RUN su - dev -c "cd /home/dev/setup && rustup toolchain install"

# Keep the dependency installers available for interactive development.
COPY script/linux/install_test_deps /home/dev/setup/script/linux/install_test_deps
COPY script/linux/install_runtime_deps /home/dev/setup/script/linux/install_runtime_deps
COPY script/linux/install_build_deps /home/dev/setup/script/linux/install_build_deps
COPY script/install_cargo_test_deps /home/dev/setup/script/install_cargo_test_deps
COPY script/install_cargo_build_deps /home/dev/setup/script/install_cargo_build_deps
COPY script/install_rust /home/dev/setup/script/install_rust

USER dev
WORKDIR /src

# Keep the detached development container running for docker exec sessions.
CMD ["sleep", "infinity"]
