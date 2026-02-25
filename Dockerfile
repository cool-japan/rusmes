# Multi-stage build for RusMES
FROM rust:1.75-slim AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build release binary
RUN cargo build --release --bin rusmes-server

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create rusmes user
RUN useradd -r -s /bin/false -u 999 rusmes

# Create directories
RUN mkdir -p /var/lib/rusmes/mailboxes \
    /var/lib/rusmes/queue \
    /var/log/rusmes \
    /etc/rusmes && \
    chown -R rusmes:rusmes /var/lib/rusmes /var/log/rusmes

# Copy binary from builder
COPY --from=builder /build/target/release/rusmes-server /usr/local/bin/

# Copy default configuration
COPY rusmes.toml /etc/rusmes/rusmes.toml
RUN chown rusmes:rusmes /etc/rusmes/rusmes.toml

# Switch to rusmes user
USER rusmes

# Expose ports
EXPOSE 25 143 587 993 8080

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s \
    CMD nc -z localhost 25 || exit 1

# Set environment
ENV RUST_LOG=info
ENV RUSMES_CONFIG=/etc/rusmes/rusmes.toml

# Run server
CMD ["/usr/local/bin/rusmes-server"]
