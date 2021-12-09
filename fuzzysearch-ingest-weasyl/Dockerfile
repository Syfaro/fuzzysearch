FROM ubuntu:20.04
RUN apt-get update -y && apt-get install -y openssl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY ./fuzzysearch-ingest-weasyl/fuzzysearch-ingest-weasyl /bin/fuzzysearch-ingest-weasyl
CMD ["/bin/fuzzysearch-ingest-weasyl"]
