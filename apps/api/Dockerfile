FROM rust:1.96-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY apps/api apps/api
COPY migrations migrations
COPY openapi openapi
RUN cargo build --release -p miz-api

FROM debian:bookworm-slim
WORKDIR /src/apps/api
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/miz-api /src/target/release/miz-api-healthcheck /src/target/release/miz-migrate /src/target/release/miz-maintain /src/target/release/miz-operator-bootstrap /usr/local/bin/
COPY --from=build /src/migrations /src/migrations
EXPOSE 8080
CMD ["miz-api"]
