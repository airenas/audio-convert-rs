#########################################################################################
## docker will invoke this file from ../.. dir in order to access code
#########################################################################################
FROM ubuntu:22.04 AS base
RUN apt-get update && \
      apt-get install -y \
      software-properties-common \
      && add-apt-repository ppa:ubuntuhandbook1/ffmpeg7
RUN apt-get update \
      && apt install -y \
      ffmpeg

RUN ffmpeg -version

#########################################################################################
FROM base AS builder
RUN apt install -y \
      clang \
      curl \
      libavcodec-dev \
      libavformat-dev \
      libavutil-dev \
      libavdevice-dev \
      libprotobuf-dev \
      libswresample-dev \
      libswscale-dev \
      protobuf-compiler \
      make

ENV RUSTFLAGS="-C target-cpu=haswell -C opt-level=3 -C codegen-units=1"
ENV CARGO_PROFILE_RELEASE_DEBUG=false      

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain 1.90.0
ENV PATH="/root/.cargo/bin:${PATH}"
RUN rustc --version && cargo --version      

# RUN apt install -y \
#       make


ARG BUILD_VERSION=0.1

WORKDIR /src/

COPY protos /src/protos
COPY build.rs /src
COPY Cargo.lock /src
COPY Cargo.toml /src
COPY src /src/src


RUN --mount=type=cache,target=/usr/local/cargo/registry \
      CARGO_APP_VERSION=$BUILD_VERSION cargo build --release 
#########################################################################################
FROM base AS runner
#########################################################################################

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
