[![Rust](https://github.com/airenas/audio-convert-rs/actions/workflows/rust.yml/badge.svg)](https://github.com/airenas/audio-convert-rs/actions/workflows/rust.yml)[![Docker](https://github.com/airenas/audio-convert-rs/actions/workflows/docker.yml/badge.svg)](https://github.com/airenas/audio-convert-rs/actions/workflows/docker.yml)

# audio-convert-rs

A lightweight Rust microservice for converting `.wav` audio files to `.mp3` and `.m4a` using FFmpeg. It provides a fast and reliable gRPC API for easy integration into media processing pipelines. The service uses the FFmpeg v7.1 C API.

## How to Compile

Refer to this [Dockerfile](build/audio-convert-rs/Dockerfile) for information on how the service code is built on Ubuntu.

### Locally

- Prepare ffmpeg library
  - Install zig
  ```bash
  curl -LO https://ziglang.org/download/0.15.2/zig-x86_64-linux-0.15.2.tar.xz \
    && tar -xf zig-x86_64-linux-0.15.2.tar.xz \
    && mv zig-x86_64-linux-0.15.2 /opt/zig \
    && ln -s /opt/zig/zig /usr/local/bin/zig
  ```
  - build ffmpeg `make prepare/ffmpeeg`
  - build service `make build/local` 


## Testing Locally

1. **Run the Service** (Default port is `50051`):
   1. Using Docker:
      ```bash
      docker run --rm -p 50051:50051 airenas/audio-convert-rs:0.1.1
      ```
   2. Using Cargo:
      ```bash
      make run
      ```

2. **Prepare a Sample File**:
   ```bash
   ffmpeg -f lavfi -i sine=frequency=1000:duration=30 -ac 1 -ar 22050 1.wav
   ```

3. **Invoke the Service**:
   1. Using the sample code:
      ```bash
      make run/client
      ```
   2. Using the sample code with streaming:
      ```bash
      make run/client stream=-s
      ```