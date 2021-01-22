FROM rust:1-slim AS builder
WORKDIR /src
RUN apt-get update -y && apt-get install -y libssl-dev pkg-config
COPY . .
RUN cargo install --root / --path .

FROM debian:buster-slim
EXPOSE 8080
WORKDIR /app
RUN apt-get update -y && apt-get install -y openssl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /bin/fuzzysearch /bin/fuzzysearch
CMD ["/bin/fuzzysearch"]
