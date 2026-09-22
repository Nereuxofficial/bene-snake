####################################################################################################
## Builder
####################################################################################################
FROM rustlang/rust:nightly AS builder

# These args are automatically provided by Docker buildx
ARG TARGETPLATFORM
ARG TARGETARCH

RUN apt update && apt install -y protobuf-compiler musl-tools
RUN update-ca-certificates

# Create appuser
ENV USER=bene-snake
ENV UID=10001

RUN adduser \
    --disabled-password \
    --gecos "" \
    --home "/nonexistent" \
    --shell "/sbin/nologin" \
    --no-create-home \
    --uid "${UID}" \
    "${USER}"

WORKDIR /bene-snake

COPY ./ .
COPY ./.env /.env

RUN case "$TARGETARCH" in \
    "amd64") RUST_TARGET="x86_64-unknown-linux-musl" ;; \
    "arm64") RUST_TARGET="aarch64-unknown-linux-musl" ;; \
    "arm")   RUST_TARGET="armv7-unknown-linux-musleabihf" ;; \
    *) echo "Unsupported architecture: $TARGETARCH" && exit 1 ;; \
    esac && \
    rustup target add "$RUST_TARGET" && \
    cargo build --release --target "$RUST_TARGET" && \
    cp "target/$RUST_TARGET/release/bene-snake" /bene-snake/bene-snake-binary


####################################################################################################
## Final image
####################################################################################################
FROM scratch

# Import from builder.
COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group

WORKDIR /bene-snake

# Copy our build
COPY --from=builder /bene-snake/bene-snake-binary ./bene-snake
COPY --from=builder /bene-snake/.env /.env

# Use an unprivileged user.
USER bene-snake:bene-snake

CMD ["/bene-snake/bene-snake"]
