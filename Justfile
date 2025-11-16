# Get the system architecture
arch := `uname -m`

# Map uname architecture to Docker TARGETARCH
docker_arch := if arch == "x86_64" {
    "amd64"
} else if arch == "aarch64" {
    "arm64"
} else if arch == "arm64" {
    "arm64"
} else if arch == "armv7l" {
    "arm"
} else {
    arch
}

# Build Docker container for current architecture
docker-build TAG="latest":
    docker build \
        --build-arg TARGETARCH={{docker_arch}} \
        -t bene-snake:{{TAG}} \
        .
