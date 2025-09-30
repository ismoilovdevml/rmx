FROM rust:1.80-slim

WORKDIR /app

RUN apt-get update && apt-get install -y \
    time \
    bc \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

RUN cp target/release/rmx /usr/local/bin/rmx

WORKDIR /test

CMD ["/bin/bash"]