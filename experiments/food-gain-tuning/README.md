# Rewarding food consumed during search

This experiment compares the current food-guided MCTS with an otherwise
identical search that adds a bonus at nonterminal rollout leaves for food eaten
since the root. The gain is measured as the increase in the search snake's
length. Terminal losses remain zero and terminal wins remain 1,000. The live
leaf reward, including the food bonus, stays below 1,000. The production
evaluator uses food-distance weight 12 and length weight 3 in both agents.

The MCTS source in `src/mcts.rs` was copied from the working-tree
`lib/src/mcts.rs` before the reward change; the candidate adds only the
bonus and the parameter used to set it. After confirmation, the same reward
rule was promoted to `lib/src/mcts.rs`. The gym replaces food after turns,
while MCTS still searches without future food spawns. The bonus gives already
eaten food lasting credit in sampled continuations.

Each seed is played in both candidate seats, with the same 150-turn two-snake
gym setup. The screen has 32 seed pairs per bonus at 10 ms per decision. The
selected bonus is tested on 128 fresh seed pairs at 50 ms. Search RNG is not
seeded; only the starting board, gym food stream, and seat assignment are
paired. Score uncertainty uses a paired seed bootstrap. Confirmed food eaten
is measured by length increases on the next observed turn, which misses food
eaten on a snake's final move. Early food is confirmed eating in its first
25 moves.

## Results

The 10 ms screen tested bonuses 20, 50, and 100. Bonus 100 scored 60.2%
(26 wins, 13 losses, 25 draws) against bonus 0, with a paired 95% interval
of 50.8–69.5%. It consumed 411 versus 377 food items overall, including
142 versus 114 in the first 25 moves, and moved toward food on 61.9% versus
60.6% of moves.

The first fresh 50 ms confirmation used 128 seed pairs and scored 52.9%
(89 wins, 74 losses, 93 draws; paired 95% interval 48.2–57.6%). Because
the point estimate and food-behavior difference were positive but the match
interval still included 50%, a further 384 new seed pairs were run unchanged.
This held-out extension scored 55.8% (300 wins, 211 losses, 257 draws;
paired 95% interval 53.0–58.7%). It ate 4,892 versus 4,393 confirmed food
items (6.37 versus 5.72 per game; paired difference interval 0.46–0.84),
including 1,860 versus 1,540 in the first 25 moves, and moved toward food
on 63.10% versus 61.23% of moves (paired difference interval 1.85–2.71
percentage points).

Across all 512 fresh 50 ms seed pairs, the candidate scored 55.1%
(paired 95% interval 52.6–57.5%). It ate 6,593 versus 5,854 food items and
moved toward food on 63.10% versus 61.07% of moves. The 384-pair extension
is the primary match-strength result because it was collected after the
128-pair result was inspected. The tested release binary SHA-256 is
`385af27a3b08cb0369e92b2af1b8febffca89ecf8cc67bef325a958bd0811e43`.

```sh
RUSTC_WRAPPER= CARGO_TARGET_DIR="$PWD/target" cargo build --release --offline --manifest-path experiments/food-gain-tuning/Cargo.toml
python3 experiments/food-gain-tuning/run_study.py screen "$PWD/target/release/bene-food-gain-tuning"
python3 experiments/food-gain-tuning/summarize.py experiments/food-gain-tuning/screen.jsonl
python3 experiments/food-gain-tuning/run_study.py confirm "$PWD/target/release/bene-food-gain-tuning" --candidate 100
python3 experiments/food-gain-tuning/summarize.py experiments/food-gain-tuning/confirm.jsonl
python3 experiments/food-gain-tuning/run_study.py extended "$PWD/target/release/bene-food-gain-tuning" --candidate 100
python3 experiments/food-gain-tuning/summarize.py experiments/food-gain-tuning/extended.jsonl
```

This is a local two-snake comparison and does not estimate leaderboard
strength.
