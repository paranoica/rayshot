# Headless GNOME Shell with software rendering, for the soak test.
FROM archlinux:latest
RUN pacman -Syu --noconfirm --needed \
        gnome-shell gjs gtk4 vulkan-swrast mesa vulkan-icd-loader wl-clipboard \
        procps-ng findutils gawk xorg-xwayland cantarell-fonts tesseract tesseract-data-eng \
    && rm -rf /var/cache/pacman/pkg/*
