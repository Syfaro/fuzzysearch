FROM debian:buster-slim
RUN apt-get update -y && apt-get install -y libssl-dev && rm -rf /var/lib/apt/lists/*
COPY ./weasyl-watcher /bin/weasyl-watcher
CMD ["/bin/weasyl-watcher"]
