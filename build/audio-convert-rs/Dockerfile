#########################################################################################
## docker will invoke this file from ../.. dir in order to access code
#########################################################################################
## versions
ARG FFMPEG_VERSION=9.0
ARG LAME_VERSION=4.0
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
      software-properties-common
      # libmp3lame-dev \
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

#####################################################
ARG FFMPEG_VERSION
ARG LAME_VERSION
ENV FFMPEG_VERSION=$FFMPEG_VERSION
ENV LAME_VERSION=$LAME_VERSION
#####################################################

RUN git clone --depth 1 --branch n$FFMPEG_VERSION https://github.com/FFmpeg/FFmpeg.git .

ENV CC="zig cc"
ENV CXX="zig c++"
ENV AR="zig ar"

# LAME built from source with CPU-specific optimizations (distro build targets baseline x86-64)
RUN mkdir /lame && cd /lame \
    && curl -fsSL https://downloads.sourceforge.net/project/lame/lame/$LAME_VERSION/lame-$LAME_VERSION.tar.gz | tar -xz --strip-components=1 \
    && RANLIB="zig ranlib" CFLAGS="-O3 -march=haswell -fno-sanitize=undefined" ./configure \
       --prefix=/ffmpeg-lib \
       --disable-shared \
       --enable-static \
       --disable-frontend \
       --disable-decoder \
       --enable-nasm \
    && make -j$(nproc) \
    && make install

RUN ./configure \
    --prefix=/ffmpeg-lib \
    --enable-gpl \
    --enable-version3 \
    --disable-shared \
    --enable-static \
    --disable-debug \
    --disable-doc \
    --enable-libmp3lame \
    --pkg-config-flags="--static" \
    --extra-cflags="-O3 -march=haswell -I/ffmpeg-lib/include" \
    --extra-ldflags="-static -L/ffmpeg-lib/lib"

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

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain 1.99.0
ENV PATH="/root/.cargo/bin:${PATH}"
RUN rustc --version && cargo --version      

# copy your FFmpeg static libs from zig build
COPY --from=ffmpeg /ffmpeg-lib/ /ffmpeg-lib

ENV LD_LIBRARY_PATH=/ffmpeg-lib/lib
ENV LIBRARY_PATH=/ffmpeg-lib/lib
ENV PKG_CONFIG_PATH=/ffmpeg-lib/lib/pkgconfig
ENV C_INCLUDE_PATH=/ffmpeg-lib/include

#####################################################
ARG FFMPEG_VERSION
ARG LAME_VERSION
ARG BUILD_VERSION=0.1
#####################################################

WORKDIR /src/

COPY protos /src/protos
COPY build.rs /src
COPY Cargo.lock /src
COPY Cargo.toml /src
COPY src /src/src


RUN --mount=type=cache,target=/usr/local/cargo/registry \
      CARGO_APP_VERSION=$BUILD_VERSION \
      CARGO_FFMPEG_VERSION=$FFMPEG_VERSION CARGO_LAME_VERSION=$LAME_VERSION \
       cargo build --release 
RUN strip /src/target/release/audio-convert-rs
#########################################################################################
FROM ghcr.io/lvillis/salus:0.1.10 AS salus
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

COPY --from=salus /bin/salus /app/salus      

ENV RUST_LOG=trace,h2=info

COPY --from=builder /src/target/release/audio-convert-rs /app/

HEALTHCHECK --start-period=3s --interval=10s --timeout=3s --retries=3 \
  CMD ["/app/salus", "grpc", "--addr", "127.0.0.1:50051"]

ENTRYPOINT ["/app/audio-convert-rs"]
