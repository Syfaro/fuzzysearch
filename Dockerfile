FROM debian:buster-slim
COPY ./weasyl-watcher /bin/weasyl-watcher
CMD ["/bin/weasyl-watcher"]
