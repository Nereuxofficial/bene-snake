# Food scoring gym comparison

This study compares the current production MCTS and gym simulator with two
evaluation functions. `baseline` uses the prior score, including its food term
only below 40 health (weight 5, or 10 below 20 health). `food-weight-12` uses
the production score, subtracting 12 points per Manhattan step to the nearest
food at every health level. All other evaluation terms and the MCTS source are
shared with the live checkout.

Each seed runs once in each seat at an equal search budget. The 64-seed screen
uses 10 ms per move; the 256 fresh-seed confirmation uses 50 ms per move.
Results are appended and fsynced one complete seat pair at a time so either
stage can resume. Search randomness is not seeded by the gym, so seat pairing
reduces but does not remove run-to-run noise.

Build and run from the repository root:

```sh
RUSTC_WRAPPER= CARGO_TARGET_DIR="$PWD/target" cargo build --release --offline --manifest-path experiments/food-scoring-study/Cargo.toml
python3 experiments/food-scoring-study/run_study.py screen "$PWD/target/release/bene-food-scoring-study"
python3 experiments/food-scoring-study/summarize.py experiments/food-scoring-study/screen.jsonl
python3 experiments/food-scoring-study/run_study.py confirm "$PWD/target/release/bene-food-scoring-study"
python3 experiments/food-scoring-study/summarize.py experiments/food-scoring-study/confirm.jsonl
```

Treat this local two-snake result as comparative evidence, not an estimate of
four-snake leaderboard win rate. A production promotion requires a clear
fresh-seed confirmation whose paired interval excludes 50%.

## Results

Completed 2026-09-28 with the current working-tree MCTS source. The 64-seed,
10 ms screen scored 55.08% for food-weight-12 (paired bootstrap 95% CI:
48.05–62.50%). On 256 fresh seeds at 50 ms, it scored 51.66% (213 wins, 196
losses, 103 draws across 512 games; paired bootstrap 95% CI: 48.34–54.98%).
The confirmation interval includes 50%, so this experiment does not show that
the stronger food score improves duel performance. The benchmark binary SHA-256
was `b8d2948fd7c26dc800b0c06d8517579798b10d33f9d388d2af6bec431ded4d46`.

## Behavioral diagnostic

To check whether the scoring change altered food collection, the study harness
also records each agent's food-eating length increases, moves directly onto a
food tile, and moves that reduce Manhattan distance to the nearest food. An
exploratory 50 ms run was stopped after 12 paired seeds (24 games) because each
pair takes about ten seconds. The candidate ate 117 food items versus 115 for
the baseline (4.88 vs. 4.79 per game); it made 1,071 moves closer to food versus
1,078 for the baseline. This small sample is descriptive only, but it suggests
the new leaf score barely changed food-seeking behavior. Results are in
`diagnostics.jsonl`.
