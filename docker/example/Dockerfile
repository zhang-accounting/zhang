FROM python:3.10.10-bullseye AS python-build
RUN pip install fire arrow
RUN curl -LJO https://raw.githubusercontent.com/zhang-accounting/zhang-example-cli/main/main.py
RUN mkdir /data
RUN python3 main.py run > /data/main.zhang


FROM rust:1.75.0 as build-env

RUN mkdir /application
WORKDIR /app
COPY ../.. /app

RUN cargo build --release --features frontend --bin zhang

FROM rust:1.74.0
LABEL org.opencontainers.image.source https://github.com/kilerd/zhang

COPY --from=python-build /data /data
COPY --from=build-env /application /application
COPY --from=build-env /app/target/release/zhang /application/zhang
RUN chmod -R go+r /data
RUN chmod -R 777 /application/zhang

RUN useradd -u 8877 zhang
USER zhang
WORKDIR application
EXPOSE 8000

ENTRYPOINT ["./zhang", "serve", "/data", "--port", "8000"]
