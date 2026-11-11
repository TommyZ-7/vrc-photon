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
        desktop-file-utils \
        docker.io \
        file \
        git \
        jq \
        libfontconfig1-dev \
        libfreetype6-dev \
        libglib2.0-bin \
        libwayland-dev \
        libx11-dev \
        libx11-xcb-dev \
        libxcb-xkb-dev \
        libxcb1-dev \
        libxkbcommon-dev \
        libxkbcommon-x11-dev \
        libvulkan-dev \
        ninja-build \
        patchelf \
        pkg-config \
        shared-mime-info \
        squashfs-tools \
        zsync \
    && rm -rf /var/lib/apt/lists/*
