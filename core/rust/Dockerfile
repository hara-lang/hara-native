# syntax=docker/dockerfile:1
FROM debian:bookworm-slim

ARG TARGETARCH
ARG VERSION=dev
LABEL org.opencontainers.image.title="hara-native" \
      org.opencontainers.image.description="HAL-free native Hara host" \
      org.opencontainers.image.source="https://github.com/hara-lang/hara-native" \
      org.opencontainers.image.version="${VERSION}"

COPY --chmod=0555 bin/${TARGETARCH}/hara-native /usr/local/bin/hara-native

USER 65532:65532
ENTRYPOINT ["/usr/local/bin/hara-native"]
