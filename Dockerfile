FROM rustlang/rust:nightly-slim AS builder
WORKDIR /src
COPY . .
RUN cargo install --root / --path .

FROM debian:buster-slim
EXPOSE 8080
WORKDIR /app
COPY --from=builder /bin/fuzzysearch /bin/fuzzysearch
CMD ["/bin/fuzzysearch"]
