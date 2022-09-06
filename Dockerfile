FROM rust:1.63 AS builder
COPY . .
RUN cargo build --release

FROM debian:buster-slim
COPY --from=builder ./target/release/docker ./target/release/docker
CMD ["/target/release/docker"]