FROM alpine:latest

LABEL org.opencontainers.image.source https://github.com/kilerd/zhang

COPY  target/x86_64-unknown-linux-musl/release/zhang /application/zhang

RUN mkdir /data

WORKDIR application
VOLUME "/data"
EXPOSE 6666

ENTRYPOINT ["./zhang", "server", "/data", "--port", "6666"]
