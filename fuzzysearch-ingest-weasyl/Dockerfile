FROM rust:1-slim-buster AS builder
WORKDIR /src
ENV SQLX_OFFLINE=true
RUN apt-get update -y && apt-get install -y libssl-dev pkg-config
COPY . .
RUN cargo install --root / --path ./fuzzysearch-ingest-weasyl

FROM rustlang/rust:nightly-slim
RUN apt-get update -y && apt-get install -y openssl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /bin/fuzzysearch-ingest-weasyl /bin/fuzzysearch-ingest-weasyl
CMD ["/bin/fuzzysearch-ingest-weasyl"]
