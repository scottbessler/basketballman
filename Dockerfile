FROM rust:1.95-slim AS build

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates
COPY static ./static

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends libssl3 ca-certificates \
    && rm -rf /var/lib/apt/lists/*

ENV DATA_PATH=/data
ENV PORT=8080

WORKDIR /app

COPY --from=build /app/target/release/basketballman /app/basketballman
COPY static ./static

ENTRYPOINT ["/app/basketballman"]
