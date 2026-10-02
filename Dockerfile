FROM rust:1 AS build
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/devy /usr/local/bin/devy
EXPOSE 8000
CMD ["devy", "serve"]
