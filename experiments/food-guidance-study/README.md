# Food-guidance gym comparison

This compares food guidance against the same MCTS implementation with the
guidance disabled. Both variants use the current production evaluator, the same
two-snake gym, 50 ms per move, and the same seed in both seat orders. Each seed
contributes one paired score: win = 1, draw = 0.5, loss = 0. The 95% interval
bootstraps paired seed scores.

The baseline disables the food prior in tree selection, the extra healthy-snake
food weights, and guided rollout sampling. It retains the existing low-health
opponent policy. The candidate enables all three changes. Search RNG is not
seeded, so paired seeds control game setup and seat while some search randomness
remains.

Build and run from the repository root:

```sh
RUSTC_WRAPPER= CARGO_TARGET_DIR="$PWD/target" cargo build --release --offline --manifest-path experiments/food-guidance-study/Cargo.toml
python3 experiments/food-guidance-study/run_study.py "$PWD/target/release/bene-food-guidance-study"
python3 experiments/food-guidance-study/summarize.py experiments/food-guidance-study/confirm.jsonl
```

The run uses 768 fresh seed pairs, with 1,536 games total. Treat results as local
duel evidence, not leaderboard win rate.

## Results

Completed 2026-09-28 with the working-tree implementation. The initial 256
seed pairs scored 51.07% (95% paired bootstrap interval 47.66–54.49%). After
adding 512 fresh pairs, all 768 pairs scored 50.88% (572 wins, 545 losses,
419 draws; 95% interval 48.93–52.87%). The larger sample narrows the interval,
but it still includes 50%, so this study does not demonstrate a win-rate
improvement. The binary used for both runs had SHA-256
`f4eeacd465cb9c121e9293b5ba10ec92633c8b0d8d32557fd96eca455849e020`.

## Combined food changes versus prior MCTS

`combined-64-50ms.jsonl` is a separate 64-seed-pair, 128-game comparison at
50 ms per move. The candidate enables both the production food-distance score
(weight 12 at all health levels) and the tree/rollout food guidance. The
baseline uses the former health-gated food score and disables the added tree and
rollout guidance. Each seed was played in both seats. The combined candidate
scored 48.83% (44 wins, 47 losses, 37 draws; paired seed-bootstrap 95% CI
41.41–56.25%), so this shorter run shows no lead. Its binary SHA-256 was
`d572ac67ae3eb81aa6cf1a1dfe3d581691415b736d7caa068d459d6824c45029`.
