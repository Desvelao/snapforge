FROM ubuntu:22.04 AS dev
ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH \
    CARGO_TARGET_DIR=/work/target
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential pkg-config curl ca-certificates git \
        libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libgl1-mesa-dev libegl1-mesa-dev \
        libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libdbus-1-dev \
        xvfb xauth libgl1-mesa-dri \
    && rm -rf /var/lib/apt/lists/*
ARG RUST_VERSION=1.88.0
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain ${RUST_VERSION} --profile minimal -c clippy -c rustfmt \
    && curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C /usr/local/cargo/bin \
    && chmod -R a+rwX /usr/local/cargo /usr/local/rustup
RUN mkdir -p /work/target /usr/local/cargo/registry && chmod -R a+rwX /work /usr/local/cargo
# Runtime libraries winit loads dynamically; present on desktops, needed here for smoke runs.
RUN apt-get update && apt-get install -y --no-install-recommends libxcursor1 libxi6 libxrandr2 libx11-6 \
    && rm -rf /var/lib/apt/lists/*
# A tiny EWMH window manager + a window to capture, for live window-capture tests.
RUN apt-get update && apt-get install -y --no-install-recommends openbox xterm x11-xserver-utils \
    && rm -rf /var/lib/apt/lists/*
# Xvnc as a virtual X server whose RandR outputs winit accepts (Xvfb's are ignored, so
# scale-factor tests cannot work on Xvfb).
RUN apt-get update && apt-get install -y --no-install-recommends tigervnc-standalone-server \
    && rm -rf /var/lib/apt/lists/*
# Coverage report (make coverage).
RUN rustup component add llvm-tools-preview && cargo install --locked cargo-llvm-cov \
    && chmod -R a+rwX /usr/local/cargo /usr/local/rustup
WORKDIR /work
# Packaging (make package): icons, desktop/AppStream validation, deb and rpm builders, arm64 cross linker.
RUN apt-get update && apt-get install -y --no-install-recommends \
        x11-utils librsvg2-bin desktop-file-utils appstream dpkg-dev rpm file xz-utils gcc-aarch64-linux-gnu libc6-dev-arm64-cross \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add aarch64-unknown-linux-gnu \
    && cargo install --locked cargo-deb cargo-generate-rpm \
    && chmod -R a+rwX /usr/local/cargo /usr/local/rustup
