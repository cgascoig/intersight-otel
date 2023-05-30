FROM rust:1.69 AS builder
COPY . .
RUN cargo build --release

FROM debian:buster-slim
RUN apt-get update && apt-get install -y openssl
COPY --from=builder ./target/release/intersight_otel ./target/release/intersight_otel
CMD ["/target/release/intersight_otel"]