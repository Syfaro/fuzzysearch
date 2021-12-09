FROM ubuntu:20.04
EXPOSE 8080
ENV METRICS_HOST=0.0.0.0:8080
RUN apt-get update -y && apt-get install -y openssl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY ./fuzzysearch-ingest-e621/fuzzysearch-ingest-e621 /bin/fuzzysearch-ingest-e621
CMD ["/bin/fuzzysearch-ingest-e621"]
