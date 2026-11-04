# Headless GNOME Shell with software rendering, for the soak test.
ARG BASE=ubuntu:24.04
FROM ${BASE}
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        gnome-shell gjs gir1.2-gtk-4.0 mesa-vulkan-drivers libvulkan1 \
        libegl-mesa0 libgl1-mesa-dri dbus dbus-user-session wl-clipboard \
        libglib2.0-bin procps findutils gawk xwayland fonts-cantarell \
    && rm -rf /var/lib/apt/lists/*
