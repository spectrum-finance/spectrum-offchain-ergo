## Builder
FROM messense/rust-musl-cross:x86_64-musl AS builder

RUN rustup target add x86_64-unknown-linux-musl
RUN apt update && apt install -y musl-tools musl-dev
RUN update-ca-certificates

# Create appuser
ENV USER=app
ENV UID=10001

RUN adduser \
    --disabled-password \
    --gecos "" \
    --home "/nonexistent" \
    --shell "/sbin/nologin" \
    --no-create-home \
    --uid "${UID}" \
    "${USER}"

WORKDIR /app

COPY ./ .

RUN cargo build --target x86_64-unknown-linux-musl --features vendored --release

## Final
FROM scratch

# Import from builder.
COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group

WORKDIR /app

# Copy our build
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/spectrum-offchain-lm ./

# Use an unprivileged user.
USER app:app

ENV RUST_BACKTRACE=1
ENTRYPOINT ["/app/spectrum-offchain-lm"]