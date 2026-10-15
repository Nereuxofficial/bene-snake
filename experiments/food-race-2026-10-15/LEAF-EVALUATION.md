# Food-race leaf evaluation

Implemented in the working tree; not committed or deployed. This is a separate comparison of leaf evaluation, with the previous root advisor enabled in both arms. Root masks, rollout/opponent policies, food-gain reward and terminal reward handling are identical.

## Scoring

Replace the nearest Manhattan food term with the minimum static route distance plus a class penalty, in equivalent steps: favourable 0, contested 2, unknown 6, likely lost 8, beyond health 12. A missing route uses Manhattan distance plus its uncertainty penalty; it is not classified as impossible. Effective distance is capped at 40 and retains the existing multiplier of 12. This lets a farther favourable meal beat a near likely-lost one, while retaining a distance gradient when every meal is adverse. These initial constants are not strength-tuned.

Recompute on each actual rollout leaf, without root target history or root distance reuse. The shared route model handles current occupancy, health, hazards, intermediate growth, rival lengths and optimistic tail uncertainty. The leaf variant avoids re-expanding equal-distance labels just to propagate unions of first directions. It uses a deterministic 2,048-expansion ceiling, no per-leaf clock reads, and falls back to the original Manhattan term if work exhausts. No food means no route analysis. Unknown or adverse estimates do not prune moves or certify death. Search constructors, child expansion, unstored rollouts and retained snapshots carry the evaluation option.

## Replay verification

The saved October 15 corpus contains three starvation games. Audited every living position from those games: 679 positions, 574 changed leaf scores. Root diagnostic work exhausted on none of those positions. The runtime cap still bounds arbitrary simulated leaves; these recorded positions do not exhaust it. Raw results are in `leaf-search.jsonl`; machine-readable counts and source hashes are in `leaf-summary.json`.

Search windows: `619cc13e` turns 66–74; `dda46e06` turns 185–193; `e6bcd8f8` turns 274–282. Each of the 27 positions received four fresh-root paired comparisons at 20,000 iterations (seeds 0–3), plus four paired 350 ms searches using 12 workers. Arm order alternates. Wall-time repeats use the production worker RNG and are not seeded pairs. Root preparation is charged to the wall-time budget; tactical proof and tree destruction are excluded. No background pondering or reused trees. This is a fixed-position search diagnostic, not complete counterfactual games.

| Comparison | Complete pairs | Different moves | Median candidate/baseline throughput |
|---|---:|---:|---:|
| 20,000 iterations | 108 | 3 | 0.630 |
| 350 ms, 12 workers | 108 | 10 | 0.763 |

Local leaf evaluation median 4.37 µs, p99 7.20 µs, maximum 10.19 µs (100-call batches over recorded boards). Equal-time candidate maximum was 351.40 ms. Measurements are local, not deployment-CPU latency evidence.

At `e6bcd8f8` turn 276, the old food term values the five-step target at (8,1) even though the longer rival reaches it simultaneously. The new term selects effective distance 10 for the favourable alternative at (2,10), reducing board evaluation from 654 to 594. Both search arms nevertheless choose **right in all four fixed-iteration and all four equal-time probes**. Recognition and leaf scoring are corrected; avoidance of the historical approach and starvation prevention are not demonstrated. At turn 278 the equal-time searches still choose right in every run. The other two games mostly retain their choices; the observed move differences have not been certified as better.

`leaf-filters.jsonl` separately replays tactical trigger continuation with the same 20,000-call/ten-turn proof limits and no clock deadline. At `dda46e06` turn 193, it restricts selection to right, whereas the unfiltered search probes choose left in both arms; this diagnostic must not be presented as the server's final move. At `e6bcd8f8` turn 281 it also excludes left, but all probes choose right. All other searched boards retain their root masks. The leaf feature cannot change these proof masks, and its toggle is tested to preserve root candidates and priors.

## Checks and limits

100 library unit tests passed, one existing diagnostic ignored; 36 server tests passed. Clippy with warnings denied and whitespace checks passed. The original full library integration suite also passed, including its timing probe; new regressions were then rerun in the library unit suite. Regressions cover score decomposition on all three starvation replays, the longer-rival versus alternative-food case, shorter rivals, empty food, dead snakes, and option propagation through expansion and retention. Existing route-model tests cover growth, hazards, last-health feeding, tails and budget exhaustion.

This implementation has a material search-throughput cost and has not established a playing-strength improvement. Keep rollout/opponent-policy changes and food-gain reward tuning as separate experiments. Do not deploy on the assumption that these three starvation losses are fixed.

## Reproduce

```bash
RUSTC_WRAPPER= cargo build --release --offline --manifest-path experiments/food-race-2026-10-15/audit/Cargo.toml --bin leaf
experiments/food-race-2026-10-15/audit/target/release/leaf experiments/recent-arena-2026-10-15 20000 350 4 1 > /tmp/leaf-search.jsonl
experiments/food-race-2026-10-15/audit/target/release/leaf experiments/recent-arena-2026-10-15 0 0 0 1 > /tmp/leaf-filters.jsonl
```
