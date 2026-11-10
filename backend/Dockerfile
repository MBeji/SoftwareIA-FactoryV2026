# syntax=docker/dockerfile:1
# OmniForge API — build context is the repository root:
#   docker build -f backend/Dockerfile -t omniforge-api .

FROM rust:1-slim-bookworm AS build
WORKDIR /src/backend
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/backend/target \
    cargo build --release --locked && cp target/release/omniforge /usr/local/bin/omniforge

FROM debian:bookworm-slim
# poppler-utils: PDF ingestion (pdftotext). The Docker CLI drives the sandbox containers.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates poppler-utils \
    && rm -rf /var/lib/apt/lists/*
COPY --from=docker:cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=build /usr/local/bin/omniforge /usr/local/bin/omniforge
COPY config/omniforge.toml /etc/omniforge/omniforge.toml

# Unprivileged user. /data is created with its ownership so that a named volume mounted
# there is writable, and so that sandboxes (started with the same uid) can write to their
# workspace sub-directory.
RUN useradd --uid 10001 --user-group --no-create-home omniforge \
    && mkdir -p /data && chown 10001:10001 /data
USER 10001:10001

ENV OMNIFORGE_DATA_DIR=/data \
    OMNIFORGE_CONFIG=/etc/omniforge/omniforge.toml \
    OMNIFORGE_BIND=0.0.0.0:8080
EXPOSE 8080
VOLUME ["/data"]
ENTRYPOINT ["omniforge"]
CMD ["serve"]
