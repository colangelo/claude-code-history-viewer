# Multi-stage build for Claude Code History Viewer (WebUI Server Mode)
# Frontend assets are embedded into the binary via rust-embed — no separate dist/ needed.

# ── Stage 1: Build frontend ──────────────────────────────────────────
FROM node:20-slim AS frontend
ARG PROXY_URL
ENV http_proxy=${PROXY_URL} https_proxy=${PROXY_URL}
RUN corepack enable
WORKDIR /app
COPY package.json pnpm-lock.yaml ./
RUN corepack install
RUN pnpm install --frozen-lockfile --prefer-offline
COPY . ./
RUN pnpm exec tsc --build . && pnpm exec vite build

# ── Stage 2: Build Rust server binary (with embedded frontend) ──────
FROM rust:1-bookworm AS backend
ARG PROXY_URL
ENV http_proxy=${PROXY_URL} https_proxy=${PROXY_URL}
# No system libraries: the viewer has no webview stack since the web-only cut (#23).
WORKDIR /app
# The whole workspace, so every member manifest resolves; only the viewer is built.
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
# rust-embed reads dist/ at compile time (crates/viewer/../../dist)
COPY --from=frontend /app/dist dist/
RUN cargo build --release -p claude-code-history-viewer --features webui-server

# ── Stage 3: Minimal runtime image ──────────────────────────────────
FROM debian:bookworm-slim
ARG PROXY_URL
# Use http_proxy only (no HTTPS rewrite needed — slim has no ca-certs yet)
ENV http_proxy=${PROXY_URL}
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
ENV http_proxy= https_proxy=

# Run as non-root user for security
RUN groupadd -r cchv && useradd -r -g cchv -d /home/cchv -s /sbin/nologin -m cchv

COPY --from=backend /app/target/release/claude-code-history-viewer /usr/local/bin/cchv-server

ENV PORT=3727
EXPOSE 3727
USER cchv

ENTRYPOINT ["cchv-server", "--serve", "--host", "0.0.0.0"]
CMD ["--port", "3727"]
