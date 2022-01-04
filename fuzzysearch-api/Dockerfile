FROM ubuntu:20.04
EXPOSE 8080 8081
ENV METRICS_HOST=0.0.0.0:8081
RUN apt-get update -y && apt-get install -y --no-install-recommends openssl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY ./fuzzysearch-api/fuzzysearch-api /bin/fuzzysearch-api
CMD ["/bin/fuzzysearch-api"]
