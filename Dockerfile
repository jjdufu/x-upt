# syntax=docker/dockerfile:1.29

FROM debian:trixie AS builder
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
RUN cargo build --release --locked \
    && test -x /src/x-upt/target/release/upupt

FROM debian:trixie-slim AS runtime
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=Asia/Shanghai
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates openssl tzdata tini \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 mazbri \
    && useradd --uid 10001 --gid 10001 --home-dir /app --shell /usr/sbin/nologin mazbri \
    && mkdir -p /app/config/keys /app/imgs /app/scripts /app/logs
WORKDIR /app
COPY --from=builder /src/x-upt/target/release/upupt /app/upupt
COPY config/ /app/config/
COPY imgs/ /app/imgs/
COPY scripts/gen_envelope_keys.sh scripts/docker-entrypoint.sh /app/scripts/
RUN chmod +x /app/scripts/gen_envelope_keys.sh /app/scripts/docker-entrypoint.sh \
    && chown -R 10001:10001 /app
USER 10001:10001
EXPOSE 3000-3008 3010-3015
HEALTHCHECK --interval=10s --timeout=3s --start-period=15s --retries=3 CMD ["bash", "-c", "exec 3<>/dev/tcp/127.0.0.1/3000"]
ENTRYPOINT ["tini", "--", "/app/scripts/docker-entrypoint.sh"]
