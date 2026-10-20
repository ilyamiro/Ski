# syntax=docker/dockerfile:1
#
# Ski in a container. Build:   docker build -t ski .
# Run (see docker/ski-docker): the window is drawn on the host's Wayland or X11 display.

FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY assets ./assets
RUN cargo build --release --locked

FROM debian:bookworm-slim
# GUI libraries are loaded at run time (Wayland, X11, OpenGL/Vulkan), plus the tools Ski shells out to
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      libwayland-client0 libxkbcommon0 libxkbcommon-x11-0 libx11-6 libx11-xcb1 libxcursor1 libxi6 libxrandr2 \
      libgl1 libegl1 libgl1-mesa-dri libvulkan1 mesa-vulkan-drivers \
      xdg-utils shared-mime-info \
      tar xz-utils bzip2 zstd p7zip-full \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/ski /usr/local/bin/ski
# Run as the caller's uid (`--user`) with their home mounted, so settings and the trash are the real ones
ENV HOME=/tmp
ENTRYPOINT ["ski"]
