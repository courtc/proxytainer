FROM rust:slim-bullseye AS build

RUN USER=root cargo new --bin app
WORKDIR /app

#COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml

RUN cargo build --release && \
	rm src/*.rs target/release/deps/proxytainer*

COPY ./src ./src

RUN cargo install --path . --profile release

FROM debian:bullseye-slim

COPY --from=build /usr/local/cargo/bin/proxytainer /usr/bin/proxytainer
ENTRYPOINT ["proxytainer"]
