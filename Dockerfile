# Use the official Rust image as the base image
FROM rust:latest

# Set environment variables
ENV PORT=8080
ENV BACKUP_INTERVAL=
ENV BACKUP_PATH=

RUN mkdir /app
WORKDIR /app

COPY ./ ./

RUN cargo build --release

CMD ["./target/release/efis"]
