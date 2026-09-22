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

# Profile a benchmark with samply
profile-bench PACKAGE BENCH PROFILE_TIME="60":
    #!/bin/bash
    cargo bench --profile profiling --package {{PACKAGE}} --bench {{BENCH}} --no-run
    BENCH_BIN=$(find target/profiling/deps -name "{{BENCH}}-*" -type f -executable | head -1)
    samply record "$BENCH_BIN" --profile-time {{PROFILE_TIME}}

# Profile the MCTS rollout benchmark specifically
profile-mcts-rollout PROFILE_TIME="60":
    just profile-bench lib mcts_rollout {{PROFILE_TIME}}

# Profile the MCTS best_child benchmark specifically
profile-mcts-best-child PROFILE_TIME="60":
    just profile-bench lib mcts_best_child {{PROFILE_TIME}}

# View the most recent samply profile
view-profile:
    samply load profile.json.gz

# Profile a benchmark and immediately open the viewer
profile-and-view PACKAGE BENCH PROFILE_TIME="10":
    just profile-bench {{PACKAGE}} {{BENCH}} {{PROFILE_TIME}}
    just view-profile
