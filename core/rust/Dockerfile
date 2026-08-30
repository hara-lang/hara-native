# syntax=docker/dockerfile:1
FROM rust:1.89-bookworm AS build

ARG VERSION
RUN test -n "$VERSION" \
    && for attempt in 1 2 3 4 5 6 7 8 9 10 11 12; do \
         cargo install --locked --version "$VERSION" hara-native \
           && break; \
         test "$attempt" = 12 && exit 1; \
         sleep 10; \
       done \
    && test -x /usr/local/cargo/bin/hara-native

FROM debian:bookworm-slim

ARG VERSION=dev
LABEL org.opencontainers.image.title="hara-native" \
      org.opencontainers.image.description="HAL-free native Hara host" \
      org.opencontainers.image.source="https://github.com/hara-lang/hara-native" \
      org.opencontainers.image.version="${VERSION}"

COPY --from=build /usr/local/cargo/bin/hara-native /usr/local/bin/hara-native

USER 65532:65532
ENTRYPOINT ["/usr/local/bin/hara-native"]
