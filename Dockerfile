FROM rust:1.68.0 as build-env
WORKDIR /app
COPY . /app

RUN mkdir /data
RUN cargo build --release --features frontend

FROM gcr.io/distroless/cc
LABEL org.opencontainers.image.source https://github.com/kilerd/zhang

COPY --from=build-env /app/target/release/zhang /application/zhang
COPY --from=build-env /data /data

WORKDIR application
VOLUME "/data"
EXPOSE 8000

ENTRYPOINT ["./zhang", "server", "/data", "--port", "8000"]
