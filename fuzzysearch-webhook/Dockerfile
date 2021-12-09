FROM ubuntu:20.04
RUN apt-get update -y && apt-get install -y openssl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY ./fuzzysearch-webhook/fuzzysearch-webhook /bin/fuzzysearch-webhook
CMD ["/bin/fuzzysearch-webhook"]
