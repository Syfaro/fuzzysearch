FROM rust:1-slim-buster AS builder
WORKDIR /src
ENV SQLX_OFFLINE=true
RUN apt-get update -y && apt-get install -y libssl-dev pkg-config
COPY . .
RUN cargo install --root / --path ./fuzzysearch-ingest-e621

FROM debian:buster-slim
EXPOSE 8080
ENV METRICS_HOST=0.0.0.0:8080
WORKDIR /app
RUN apt-get update -y && apt-get install -y openssl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /bin/fuzzysearch-ingest-e621 /bin/fuzzysearch-ingest-e621
CMD ["/bin/fuzzysearch-ingest-e621"]
