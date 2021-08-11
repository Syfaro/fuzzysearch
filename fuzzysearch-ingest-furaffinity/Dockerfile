FROM debian:buster-slim
RUN apt-get update -y && \
    apt-get install -y openssl ca-certificates python3 python3-pip && \
    python3 -m pip --no-cache-dir install cfscrape && \
    rm -rf /var/lib/apt/lists/*
COPY ./fuzzysearch-ingest-furaffinity/fuzzysearch-ingest-furaffinity /bin/fuzzysearch-ingest-furaffinity
CMD ["/bin/fuzzysearch-ingest-furaffinity"]
