FROM ubuntu:24.04
RUN apt-get update -y && apt-get install -y openssl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY ./fuzzysearch-refresh/fuzzysearch-refresh /bin/fuzzysearch-refresh
CMD ["/bin/fuzzysearch-refresh"]
