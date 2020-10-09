FROM debian:buster-slim
RUN apt-get update -y && apt-get install -y libssl1.0.0 && rm -rf /var/lib/apt/lists/*
COPY ./weasyl-watcher /bin/weasyl-watcher
CMD ["/bin/weasyl-watcher"]
