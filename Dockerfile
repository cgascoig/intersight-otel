FROM rust:1.84 AS builder
COPY . .
RUN cargo build --release

FROM debian:buster-slim
RUN apt-get update && apt-get install -y openssl ca-certificates
COPY --from=builder ./target/release/intersight_otel ./target/release/intersight_otel
CMD ["/target/release/intersight_otel"]