FROM rust:1.75.0 as build-env
WORKDIR /app
COPY . /app

RUN mkdir /data
RUN cargo build --release --features frontend --bin zhang

FROM gcr.io/distroless/cc-debian12
LABEL org.opencontainers.image.source https://github.com/kilerd/zhang

COPY --from=build-env /app/target/release/zhang /application/zhang
COPY --from=build-env /data /data

ENV ZHANG_AUTH=""
WORKDIR application
VOLUME "/data"
EXPOSE 8000

ENTRYPOINT ["./zhang", "serve", "/data", "--port", "8000"]
