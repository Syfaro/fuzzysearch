FROM rust:1-slim-buster AS builder
WORKDIR /src
ENV SQLX_OFFLINE=true
RUN apt-get update -y && apt-get install -y libssl-dev pkg-config python3 python3-pip
COPY . .
RUN cargo install --root / --path ./fuzzysearch-ingest-furaffinity

FROM rustlang/rust:nightly-slim
RUN apt-get update -y && \
    apt-get install -y openssl ca-certificates python3 python3-pip && \
    python3 -m pip --no-cache-dir install cfscrape && \
    rm -rf /var/lib/apt/lists/*
COPY --from=builder /bin/fuzzysearch-ingest-furaffinity /bin/fuzzysearch-ingest-furaffinity
CMD ["/bin/fuzzysearch-ingest-furaffinity"]
