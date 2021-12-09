FROM ubuntu:20.04
RUN apt-get update -y && apt-get install -y openssl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY ./fuzzysearch-hash-input/fuzzysearch-hash-input /bin/fuzzysearch-hash-input
CMD ["/bin/fuzzysearch-hash-input"]
