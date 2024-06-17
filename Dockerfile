FROM gcr.io/distroless/cc-debian12
LABEL org.opencontainers.image.source https://github.com/kilerd/zhang


COPY --chmod=755 zhang /application/

ENV ZHANG_AUTH=""
ENV RUST_LOG="info"
WORKDIR application
VOLUME "/data"
EXPOSE 8000

ENTRYPOINT ["./zhang", "serve", "/data", "--port", "8000"]
