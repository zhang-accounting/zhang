FROM alpine:latest

LABEL org.opencontainers.image.source https://github.com/kilerd/zhang

COPY  target/x86_64-unknown-linux-musl/release/zhang /application/zhang

WORKDIR application
VOLUME "/application/data"
EXPOSE 8000

ENTRYPOINT ["./zhang", "server", "/application/data", "--port", "8000"]
