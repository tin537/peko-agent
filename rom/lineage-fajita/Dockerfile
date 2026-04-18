# Dockerfile — Reproducible LineageOS build host for peko.
#
# Build:
#   docker buildx build --platform linux/amd64 -t peko-builder \
#     -f rom/lineage-fajita/Dockerfile rom/lineage-fajita/
#
# Use:
#   # One-time: create a named volume so the 80GB source survives restarts
#   docker volume create lineage-src
#
#   # Open a shell in the builder (bind-mount this repo at /work/peko)
#   docker run -it --rm \
#     --platform linux/amd64 \
#     -v lineage-src:/home/builder/lineage \
#     -v $(pwd):/work/peko \
#     -w /work/peko \
#     peko-builder
#
#   # Inside:
#   ./rom/lineage-fajita/build.sh --init    # first run only
#   ./rom/lineage-fajita/build.sh           # build the ROM
#
# Notes:
#   - Apple Silicon: force `--platform linux/amd64` (Rosetta emulates).
#     Slower but avoids the ARM64-prebuilt-missing pitfalls in LOS 21.
#   - Intel Mac / x86 Linux: drop --platform, runs native.
#   - Give Docker Desktop ≥ 16GB RAM, ≥ 250GB disk. AOSP OOM-kills at 8GB.

FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=UTC

# ─── AOSP build dependencies ────────────────────────────────
RUN apt-get update && apt-get install -y --no-install-recommends \
      bc bison build-essential ccache curl flex g++-multilib \
      gcc-multilib git git-lfs gnupg gperf imagemagick \
      lib32ncurses5-dev lib32z1-dev libelf-dev liblz4-tool \
      libncurses5 libncurses5-dev libsdl1.2-dev libssl-dev \
      libxml2 libxml2-utils lzop pngcrush rsync schedtool \
      squashfs-tools xsltproc zip zlib1g-dev \
      python3 python3-pip python-is-python3 \
      openjdk-17-jdk-headless \
      openssh-client ca-certificates sudo \
      unzip cmake pkg-config \
    && rm -rf /var/lib/apt/lists/*

# ─── `repo` tool ────────────────────────────────────────────
RUN curl -fsSL https://storage.googleapis.com/git-repo-downloads/repo \
      -o /usr/local/bin/repo \
    && chmod +x /usr/local/bin/repo

# ─── Rust (peko-agent cross-compile) ────────────────────────
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
ENV PATH=/opt/cargo/bin:${PATH}
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain stable --no-modify-path \
    && rustup target add aarch64-linux-android \
    && chmod -R a+rwX /opt/rustup /opt/cargo

# ─── Android NDK (peko-llm-daemon cross-compile) ────────────
ARG NDK_VERSION=27.1.12297006
ENV ANDROID_NDK_ROOT=/opt/android-ndk-${NDK_VERSION}
ENV ANDROID_NDK_HOME=${ANDROID_NDK_ROOT}
ENV PATH=${ANDROID_NDK_ROOT}/toolchains/llvm/prebuilt/linux-x86_64/bin:${PATH}
RUN curl -fsSL "https://dl.google.com/android/repository/android-ndk-r27c-linux.zip" \
      -o /tmp/ndk.zip \
    && unzip -q /tmp/ndk.zip -d /opt \
    && mv /opt/android-ndk-r27c "${ANDROID_NDK_ROOT}" \
    && rm /tmp/ndk.zip

# ─── Non-root builder user (AOSP complains if you build as root) ─
RUN useradd -m -u 1000 -s /bin/bash builder \
    && echo 'builder ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/builder

USER builder
WORKDIR /home/builder

# git user (repo tool refuses without it)
RUN git config --global user.name  "peko builder" \
    && git config --global user.email "peko@localhost" \
    && git config --global color.ui auto

# ccache speeds up subsequent builds 5-10x — persist it in the volume
ENV USE_CCACHE=1
ENV CCACHE_DIR=/home/builder/.ccache
ENV CCACHE_EXEC=/usr/bin/ccache
RUN ccache -M 50G

CMD ["/bin/bash"]
