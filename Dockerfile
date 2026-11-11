# Shared Linux CI image for the GPUI build and release packaging jobs.
#
# Rust is intentionally not installed here: each job keeps rustup so the
# toolchain and cache actions behave exactly as they do on the host runner.
# The image tag is fixed; bump it in the workflow references (ci.yml,
# release.yml) whenever the package set below changes.
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        clang \
        cmake \
        curl \
        docker.io \
        file \
        git \
        jq \
        libfontconfig1-dev \
        libfreetype6-dev \
        libwayland-dev \
        libx11-dev \
        libx11-xcb-dev \
        libxcb-xkb-dev \
        libxcb1-dev \
        libxkbcommon-dev \
        libxkbcommon-x11-dev \
        libvulkan-dev \
        ninja-build \
        pkg-config \
        squashfs-tools \
    && rm -rf /var/lib/apt/lists/*
