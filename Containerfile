# Build environment for `cargo dist` (see xtask/). Debian's glibc here is the minimum glibc
# the release binaries need. aarch64 is cross-compiled against Debian's arm64 multiarch packages.
FROM docker.io/library/rust:trixie

RUN dpkg --add-architecture arm64 && \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        gcc-aarch64-linux-gnu \
        libc6-dev-arm64-cross \
        libfontconfig-dev \
        libfontconfig-dev:arm64 \
        libxcb1-dev \
        libxcb1-dev:arm64 \
        libxkbcommon-dev \
        libxkbcommon-dev:arm64 \
        libxkbcommon-x11-dev \
        libxkbcommon-x11-dev:arm64 \
        pkgconf && \
    rm -rf /var/lib/apt/lists/*

RUN rustup target add aarch64-unknown-linux-gnu

ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    PKG_CONFIG_ALLOW_CROSS_aarch64_unknown_linux_gnu=1 \
    PKG_CONFIG_LIBDIR_aarch64_unknown_linux_gnu=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig
