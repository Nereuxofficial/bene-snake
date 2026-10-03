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

# Run the Snake Gym CLI (for example: just gym tournament --games 100)
gym *ARGS:
    cargo run --release --package gym -- {{ARGS}}

# Evaluate scoring-function variants against the downloaded replay positions
bench-scoring *ARGS:
    RUSTC_WRAPPER= cargo run --release --package lib --example evaluation_score_benchmark -- {{ARGS}}

# Run the focused timing suite (optional arguments go to Criterion)
bench *ARGS:
    cargo bench --package lib --features bench --bench mcts_rollout --bench mcts_expand --bench mcts_hotpaths --bench mcts_best_child -- {{ARGS}}

# Count allocations and bytes per operation separately from timing benchmarks
bench-alloc *ARGS:
    cargo bench --package lib --features bench --bench mcts_allocations -- {{ARGS}}

# Profile a benchmark with samply
profile-bench PACKAGE BENCH PROFILE_TIME="60" FILTER="":
    #!/bin/bash
    set -euo pipefail
    profile_build_log=$(mktemp)
    trap 'rm -f "$profile_build_log"' EXIT
    profile_features=()
    if [[ "{{PACKAGE}}" == "lib" ]]; then profile_features=(--features bench); fi
    cargo bench --profile profiling --package {{PACKAGE}} --bench {{BENCH}} "${profile_features[@]}" --no-run --message-format=json > "$profile_build_log"
    BENCH_BIN=$(python3 -c 'import json,sys; rows=[json.loads(line) for line in open(sys.argv[1]) if line.strip()]; print(next(r["executable"] for r in reversed(rows) if r.get("reason")=="compiler-artifact" and r["target"]["name"]==sys.argv[2] and r.get("executable")))' "$profile_build_log" "{{BENCH}}")
    samply record "$BENCH_BIN" --profile-time {{PROFILE_TIME}} "{{FILTER}}"

# Profile the MCTS rollout benchmark specifically
profile-mcts-rollout PROFILE_TIME="60" FILTER="rollout_production_rng/four_crowded":
    just profile-bench lib mcts_rollout {{PROFILE_TIME}} "{{FILTER}}"

# Profile the MCTS best_child benchmark specifically
profile-mcts-best-child PROFILE_TIME="60" FILTER="best_move_warm_tree/four_crowded":
    just profile-bench lib mcts_best_child {{PROFILE_TIME}} "{{FILTER}}"

# View the most recent samply profile
view-profile:
    samply load profile.json.gz

# Profile a benchmark and immediately open the viewer
profile-and-view PACKAGE BENCH PROFILE_TIME="10" FILTER="":
    just profile-bench {{PACKAGE}} {{BENCH}} {{PROFILE_TIME}} "{{FILTER}}"
    just view-profile

# Capture the release MCTS hot loop and open it in Tracy (duration in seconds)
[positional-arguments]
profile-tracy SECONDS="15":
    #!/usr/bin/env bash
    set -euo pipefail
    seconds="$1"
    [[ "$seconds" =~ ^[1-9][0-9]*$ ]] || { echo "Duration must be a positive integer in seconds" >&2; exit 1; }
    for tool in tracy-capture tracy-profiler timeout; do
        command -v "$tool" >/dev/null || { echo "Missing command: $tool" >&2; exit 1; }
    done
    RUSTC_WRAPPER= cargo build --release --package lib --example tracy_hot_loop --features bench,tracy
    mkdir -p target/tracy
    capture_dir=$(mktemp -d "$PWD/target/tracy/hot-loop-XXXXXXXX")
    capture="$capture_dir/hot-loop.tracy"
    workload_pid=""
    cleanup() {
        if [[ -n "$workload_pid" ]]; then
            kill "$workload_pid" 2>/dev/null || true
            wait "$workload_pid" 2>/dev/null || true
        fi
    }
    trap cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    target/release/examples/tracy_hot_loop "$seconds" &
    workload_pid=$!
    timeout --foreground "$((10#$seconds + 70))s" tracy-capture -a 127.0.0.1 -o "$capture"
    wait "$workload_pid"
    workload_pid=""
    echo "Capture saved to $capture"
    tracy-profiler "$capture"
