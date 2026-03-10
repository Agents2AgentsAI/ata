FROM rust:1-slim AS builder
WORKDIR /src
COPY . .
RUN cargo build --release -p codex-coordination-relay

FROM debian:bookworm-slim
COPY --from=builder /src/target/release/codex-relay /usr/local/bin/
EXPOSE 7800
ENTRYPOINT ["codex-relay"]
