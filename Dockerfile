####################################################################################################
## Builder
####################################################################################################
FROM rustlang/rust:nightly AS builder

# These args are automatically provided by Docker buildx
ARG TARGETPLATFORM
ARG TARGETARCH

RUN apt update && apt install -y protobuf-compiler musl-tools
RUN update-ca-certificates

# Install cross-compilation tools for different architectures
RUN if [ "$TARGETARCH" = "arm64" ]; then \
    apt install -y gcc-aarch64-linux-gnu musl-dev; \
    fi

# Add musl targets for different architectures
RUN case "$TARGETARCH" in \
    "amd64") rustup target add x86_64-unknown-linux-musl ;; \
    "arm64") rustup target add aarch64-unknown-linux-musl ;; \
    "arm") rustup target add armv7-unknown-linux-musleabihf ;; \
    *) echo "Unsupported architecture: $TARGETARCH" && exit 1 ;; \
    esac

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

# Set up cross-compilation environment and build for the target architecture
RUN case "$TARGETARCH" in \
    "amd64") \
    export RUST_TARGET="x86_64-unknown-linux-musl" \
    ;; \
    "arm64") \
    export RUST_TARGET="aarch64-unknown-linux-musl" && \
    export CC=aarch64-linux-gnu-gcc && \
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-gnu-gcc \
    ;; \
    "arm") \
    export RUST_TARGET="armv7-unknown-linux-musleabihf" \
    ;; \
    esac && \
    cargo build --release --target $RUST_TARGET && \
    cp target/$RUST_TARGET/release/bene-snake /bene-snake/bene-snake-binary


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
