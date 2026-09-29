# Food-distance and length-weight tuning

This experiment uses the uncommitted 2026-09-28 food-guided MCTS as the baseline.
Only the leaf evaluator's food-distance and length weights vary. The incumbent
uses food weight 12 at all health levels and length weight 3. The gym replaces
food after turns, while the MCTS tree and rollouts do not model future spawns.

Every configuration faces the incumbent on the same randomly generated gym
seeds in both seats, using 150-turn two-snake games. The 24-seed screen uses
10 ms per decision. Its best result is checked on 128 fresh seeds at 50 ms.
The search RNG is not seeded, so setup/seat pairing does not make games fully
deterministic. The paired bootstrap interval resamples seed pairs.

The harness records moves that reduce Manhattan distance to the nearest food,
moves onto a food cell, and confirmed eating as increases in snake length on
the next observed turn. The latter misses an item eaten on a snake's final move.
"Early food" is confirmed eating in a snake's first 25 moves.

## Results

The screen tested food/length weights 12/8, 12/20, 24/3, 24/8, and 36/8.
The best match score was 24/8 at 58.3% over 48 games, but its paired 95%
interval was 46.9–68.8%. In the fresh confirmation, 24/8 scored 51.8%
(86 wins, 77 losses, 93 draws; paired 95% interval 46.9–56.6%). It ate
1,461 items versus the baseline's 1,490 and made 61.2% of moves toward food
versus 61.7% for the baseline. Early confirmed food was 524 versus 512, with
a paired difference interval including zero. There is no evidence that these
weight increases improve either strength or food seeking at 50 ms, so the
production weights remain 12/3.

The release binary SHA-256 was
`5e9e5c569a5fa35030aa19b9ee397652855cfbba6ff3f0fbdee310a822bec772`.

```sh
RUSTC_WRAPPER= CARGO_TARGET_DIR="$PWD/target" cargo build --release --offline --manifest-path experiments/food-weight-tuning/Cargo.toml
python3 experiments/food-weight-tuning/run_study.py screen "$PWD/target/release/bene-food-weight-tuning"
python3 experiments/food-weight-tuning/summarize.py experiments/food-weight-tuning/screen.jsonl
python3 experiments/food-weight-tuning/run_study.py confirm "$PWD/target/release/bene-food-weight-tuning" --candidate 24,8
python3 experiments/food-weight-tuning/summarize.py experiments/food-weight-tuning/confirm.jsonl
```

This is local two-snake evidence, not an estimate of four-snake leaderboard
performance.
