FROM rust:1.99.1-alpine3.24 AS build
RUN apk add --no-cache musl-dev pkgconfig cmake make g++
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY web ./web
RUN cargo build --locked --release --bin mail

FROM alpine:3.24
RUN apk add --no-cache ca-certificates
COPY --from=build /build/target/release/mail /usr/local/bin/mail
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint
RUN chmod 0755 /usr/local/bin/docker-entrypoint
EXPOSE 8080 8099
ENTRYPOINT ["/usr/local/bin/docker-entrypoint"]
