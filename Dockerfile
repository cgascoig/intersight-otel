FROM rust:1.63 AS builder
COPY . .
RUN cargo build --release

FROM debian:buster-slim
COPY --from=builder ./target/release/intersight_metrics ./target/release/intersight_metrics
CMD ["/target/release/intersight_metrics"]