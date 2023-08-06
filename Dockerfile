# Stage 1: Build the Efis binary
FROM rust:latest as builder

WORKDIR /usr/src/efis

# Install the required tools for building
RUN apt-get update && \
    apt-get install -y musl-tools

# Copy the project files
COPY . .

# Build the Efis binary with --release flag
RUN rustup target add x86_64-unknown-linux-musl
RUN cargo build --release --target x86_64-unknown-linux-musl

# Stage 2: Create the final image
FROM debian:buster-slim

# Set environment variables
ENV PORT=8080 \
    BACKUP_PATH=./backup \
    BACKUP_INTERVAL=120

WORKDIR /app

# Copy the binary from the builder stage
COPY --from=builder /usr/src/efis/target/x86_64-unknown-linux-musl/release/efis .

# Expose the port on which Efis listens
EXPOSE $PORT

# Run the Efis server
CMD ["./efis"]