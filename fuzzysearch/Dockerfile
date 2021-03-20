FROM rust:1-slim-buster AS builder
WORKDIR /src
ENV SQLX_OFFLINE=true
RUN apt-get update -y && apt-get install -y libssl-dev pkg-config libavcodec-dev libavformat-dev libavutil-dev libavdevice-dev clang llvm
COPY . .
RUN cargo install --root / --path ./fuzzysearch

FROM debian:buster-slim
EXPOSE 8080 8081
ENV METRICS_HOST=0.0.0.0:8081
WORKDIR /app
RUN apt-get update -y && apt-get install -y --no-install-recommends openssl ca-certificates ffmpeg && rm -rf /var/lib/apt/lists/*
COPY --from=builder /bin/fuzzysearch /bin/fuzzysearch
CMD ["/bin/fuzzysearch"]
