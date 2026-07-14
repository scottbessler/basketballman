FROM rust:1.95-slim AS build

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates

RUN cargo build --release

FROM debian:bookworm-slim

ENV DATA_PATH=/data
ENV PORT=8080

WORKDIR /app

COPY --from=build /app/target/release/basketballman /app/basketballman
COPY static ./static

ENTRYPOINT ["/app/basketballman"]
