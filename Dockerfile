FROM rustlang/rust:nightly-slim
COPY . .
RUN apt-get -y update && apt-get -y install pkg-config libssl-dev
RUN cargo install --root / --path .
CMD ["/bin/fa-watcher"]
