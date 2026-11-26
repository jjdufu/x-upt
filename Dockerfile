# syntax=docker/dockerfile:1.29

FROM node:24-bookworm-slim AS web-builder
WORKDIR /src/web
RUN npm install -g pnpm@12.21.0
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build && test -s /src/web/dist/index.html

FROM debian:trixie AS rust-builder
ARG CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=${CARGO_REGISTRIES_CRATES_IO_PROTOCOL}
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain none --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"
WORKDIR /src/x-upt
COPY rust-toolchain.toml ./
RUN rustup toolchain install && rustc -V
COPY . /src/x-upt
COPY --from=web-builder /src/web/dist /src/x-upt/web/dist
RUN cargo build --release --locked \
    && test -x /src/x-upt/target/release/x-upt

FROM debian:trixie-slim AS runtime
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=Asia/Shanghai
ENV X_UPT_ADMIN_LISTEN=0.0.0.0:18768
ENV X_UPT_DATA_DIR=/data
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl openssl tzdata tini \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 xupt \
    && useradd --uid 10001 --gid 10001 --home-dir /app --shell /usr/sbin/nologin xupt \
    && mkdir -p /app/scripts /data \
    && chown -R 10001:10001 /app /data
WORKDIR /app
COPY --from=rust-builder /src/x-upt/target/release/x-upt /app/x-upt
COPY scripts/docker-entrypoint.sh /app/scripts/docker-entrypoint.sh
RUN chmod +x /app/x-upt /app/scripts/docker-entrypoint.sh \
    && chown -R 10001:10001 /app /data
USER 10001:10001
VOLUME ["/data"]
EXPOSE 18768 16000-16008 16010-16015
HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD ["curl", "-fsS", "http://127.0.0.1:18768/health"]
ENTRYPOINT ["tini", "--", "/app/scripts/docker-entrypoint.sh"]
