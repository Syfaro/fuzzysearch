FROM debian:buster-slim
EXPOSE 8080 8081
ENV METRICS_HOST=0.0.0.0:8081
RUN apt-get update -y && apt-get install -y --no-install-recommends openssl ca-certificates ffmpeg && rm -rf /var/lib/apt/lists/*
COPY ./fuzzysearch/fuzzysearch /bin/fuzzysearch
CMD ["/bin/fuzzysearch"]
