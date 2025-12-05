#########################################################################################
## docker will invoke this file from ../.. dir in order to access code
#########################################################################################
FROM ubuntu:22.04 AS base
#########################################################################################
## base builder
#########################################################################################
FROM base AS base-builder
RUN apt-get update && \
      apt-get install -y \
      curl \
      git \
      pkg-config \
      libmp3lame-dev \
      software-properties-common
#########################################################################################
## build FFmpeg with static libs for linking into final binary
#########################################################################################
FROM base-builder AS ffmpeg

RUN apt-get install -y \
    build-essential \
    nasm \
    yasm \
    zlib1g-dev

# install zig
RUN curl -LO https://ziglang.org/download/0.15.2/zig-x86_64-linux-0.15.2.tar.xz \
    && tar -xf zig-x86_64-linux-0.15.2.tar.xz \
    && mv zig-x86_64-linux-0.15.2 /opt/zig \
    && ln -s /opt/zig/zig /usr/local/bin/zig
# verify zig installation
RUN zig version

WORKDIR /ffmpeg

ENV FFMPEG_VERSION=7.1.3

RUN git clone --depth 1 --branch n$FFMPEG_VERSION https://github.com/FFmpeg/FFmpeg.git .

ENV CC="zig cc"
ENV CXX="zig c++"
ENV AR="zig ar"

RUN ./configure \
    --prefix=/ffmpeg-lib \
    --enable-gpl \
    --enable-version3 \
    --disable-shared \
    --enable-static \
    --disable-debug \
    --disable-doc \
    --enable-libmp3lame \
    --extra-cflags="-O3 -march=haswell" \
    --extra-ldflags="-static"

RUN make -j$(nproc) \
      && make install

#########################################################################################
FROM base-builder AS builder
RUN apt-get install -y \
      binutils \
      clang \
      libprotobuf-dev \
      protobuf-compiler \
      perl \
      make

ENV RUSTFLAGS="-C target-cpu=haswell -C opt-level=3 -C codegen-units=1"
ENV CARGO_PROFILE_RELEASE_DEBUG=false      

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain 1.90.0
ENV PATH="/root/.cargo/bin:${PATH}"
RUN rustc --version && cargo --version      

# copy your FFmpeg static libs from zig build
COPY --from=ffmpeg /ffmpeg-lib/ /ffmpeg-lib

ENV LD_LIBRARY_PATH=/ffmpeg-lib/lib
ENV LIBRARY_PATH=/ffmpeg-lib/lib
ENV PKG_CONFIG_PATH=/ffmpeg-lib/lib/pkgconfig
ENV C_INCLUDE_PATH=/ffmpeg-lib/include

ARG BUILD_VERSION=0.1

WORKDIR /src/

COPY protos /src/protos
COPY build.rs /src
COPY Cargo.lock /src
COPY Cargo.toml /src
COPY src /src/src


RUN --mount=type=cache,target=/usr/local/cargo/registry \
      CARGO_APP_VERSION=$BUILD_VERSION cargo build --release 
RUN strip /src/target/release/audio-convert-rs
#########################################################################################
FROM base AS runner
#########################################################################################
RUN apt-get update && \
    apt-get install -y --no-install-recommends libmp3lame0 && \
    rm -rf /var/lib/apt/lists/*

COPY LICENSE /licenses/LICENSE-gnu

WORKDIR /app

ARG BUILD_VERSION=0.1

EXPOSE 50051

LABEL org.opencontainers.image.version=$BUILD_VERSION \
      org.opencontainers.image.authors="airenass@gmail.com" \
      maintainer="airenass@gmail.com" \
      version=$BUILD_VERSION \
      release=$BUILD_VERSION \
      summary="This image is used to host audio converter service" \
      description="This image is used to host audio converter service" 

ENV RUST_LOG=trace,h2=info

COPY --from=builder /src/target/release/audio-convert-rs /app/

ENTRYPOINT ["/app/audio-convert-rs"]
