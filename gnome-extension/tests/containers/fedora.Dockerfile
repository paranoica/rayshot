# Headless GNOME Shell with software rendering, for the soak test.
ARG BASE=fedora:43
FROM ${BASE}
# Releases past their end of life only remain in the archive.
ARG ARCHIVED=
RUN if [ -n "$ARCHIVED" ]; then sed -i -e 's|^metalink=|#metalink=|' \
        -e 's|^#baseurl=http://download.example/pub/fedora/linux|baseurl=https://archives.fedoraproject.org/pub/archive/fedora/linux|' \
        /etc/yum.repos.d/*.repo && rm -f /etc/yum.repos.d/fedora-cisco-openh264.repo; fi
RUN dnf -y install --setopt=install_weak_deps=False \
        gnome-shell gjs gtk4 mesa-vulkan-drivers mesa-dri-drivers vulkan-loader \
        dbus-daemon dbus-tools wl-clipboard glib2 procps-ng findutils gawk \
        xorg-x11-server-Xwayland abattis-cantarell-fonts \
    && dnf clean all
