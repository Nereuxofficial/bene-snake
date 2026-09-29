# MCTS tree reuse study

The server retains visited children of the move it sent. On the next `/move`, it
reuses a child only when the game ID, turn, snake ID, snake length, and complete
board match. A different food spawn or any other state difference starts a new
tree. The cache holds at most 32 response candidates and 20,000 retained visits
per game, across at most 16 games for 90 seconds.

The study compares this policy with a fresh tree on every move. Both agents use
the same current MCTS, each receives the same search time, and each seed is run
twice with the seats swapped. Every game is capped at 150 turns. The cache and
counters reset between games. The simulator supplies food respawns, which is
relevant to whether the exact-state match activates.

```sh
RUSTC_WRAPPER= CARGO_TARGET_DIR="$PWD/target" cargo build --release --offline --manifest-path experiments/tree-reuse-study/Cargo.toml
python3 experiments/tree-reuse-study/run_study.py screen "$PWD/target/release/bene-tree-reuse-study"
python3 experiments/tree-reuse-study/run_study.py confirm "$PWD/target/release/bene-tree-reuse-study"
python3 experiments/tree-reuse-study/run_study.py overhead "$PWD/target/release/bene-tree-reuse-study"
python3 experiments/tree-reuse-study/summarize.py experiments/tree-reuse-study/confirm.jsonl
```

The 32-pair, 10 ms screen and 128-pair, 50 ms confirmation used the pre-overhead
instrumentation executable (SHA-256
`7ee7a6c7a506554e5adf32f89f3d68e9b5fb344ff9fe298e1ac90bd93ac37ceb`).
The overhead run uses the same search code with additional timers around the
whole call and cache bookkeeping. JSONL results are saved alongside this file.

| Run | Pairs / games | Reuse score (paired 95% bootstrap CI) | Cache hits | Carried visits / move | Effective root visits / move (reuse vs fresh) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Screen, 10 ms | 32 / 64 | 59.38% [49.22%, 68.75%] | 5101 / 6582 (77.50%) | 238.9 | 1343.4 vs 899.4 |
| Confirm, 50 ms | 128 / 256 | 50.98% [46.29%, 55.66%] | 21198 / 27424 (77.30%) | 1132.7 | 6730.2 vs 5352.5 |

Effective visits add retained historical visits to new iterations. They measure
how much searched state the next call can use, not iterations per second. The
agents follow different trajectories, so new iterations per move cannot be
interpreted as a direct throughput comparison. The confirmation interval
includes 50%; these results do not establish a playing-strength gain. This is
a two-snake gym comparison, not a four-snake or live-server strength test.

The separate 16-pair, 50 ms overhead run used executable SHA-256
`dd1ba08a9dae09a8daff47ca0fb34a1c4c03db8ec13d18b53310d7fe9b658c9f`.
It matched 2,794 of 3,631 moves (76.95%). Whole move calls averaged 50.231 ms
with reuse and 50.101 ms with fresh trees. Cache lookup, including disposal of
unmatched candidates, averaged 129.3 microseconds per reuse move; cache
preparation rounded below one microsecond per move at the timer resolution.
