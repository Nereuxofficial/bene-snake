# Enclosure evaluator: frozen precursor screen

Implemented by the enclosure subagent, reviewed and corrected afterward by the parent. The initial results below describe the subagent snapshot in `summary.json`, before the parent's final fix pass. No commit, deployment, Arena strength claim, or claimed rescue.

The feature subtracts at most96 score units for usable space smaller than current body length. There is no bonus for owning extra territory. Static rival arrivals gate cells only if an equal/longer arrival-length rival can get there at least as early. Rival threat routes are bounded to12 turns; own feasible arrival coverage spans the board. Both account for health, hazards, food, free tails and first-step neck legality. Regions cannot claim access through a lethal hazard. Growth or tail-release uncertainty halves the penalty. A single shortest static route label per cell deliberately leaves dynamic and longer alternatives unresolved; this is a soft estimate, never a survival proof or move filter. The scratch arrays stay on the stack; at most four121-cell arrival passes and one121-cell region pass, with no per-leaf clock reads.

`evaluate_board_with_options(...,food_races,enclosure)` and `Node::new_root_with_enclosure_evaluation` isolate this arm. The historical `evaluate_board_with_food_races` API remains enclosure-free. Existing food advice, rollout actions, terminal rewards, root masks and tactical certification are unchanged. The subagent initially enabled the term in normal Node/evaluate defaults so it could be independently reviewed; the parent owns the final activation decision.

## Fixtures and protocol

The51 recorded precursor boards come from `c7c9a459` turns228–243, `6bba0171` turns228–243 and `325da80a` turns252–270. Dead replay snakes were excluded. They include states before the first exposed root candidates. Source replay files remain in `../postdeploy-8ccfd2fb-2026-10-15/`.

The measured screen was declared before execution:13 boards every four turns (four per first two games, five in the third), fresh roots and3 comparisons per board, alternating arm order. Food races stay enabled in both arms. First10,000 deterministic single-thread iterations with seeds0..2, then60ms total per arm with those seeds. Finally350ms total,12 workers,3 independent repeats per arm, using unseeded worker RNG; these repetitions are not seeded pairs. Root construction/preparation is charged to the budget. Results include worker completion but exclude tree destruction; production tactical prep and HTTP delivery are not simulated. Calibration was a separate single60ms pair, showing roughly16% fewer iterations.

## Results

- All51 static boards:22 receive a nonzero penalty,17 have a contested frontier. Assessment mean1.22 microseconds on this local run.
-39 fixed-iteration pairs:2 different decisions, both `325da80a` turn252, seeds0 and1, Left to Up. The third seed already chose Up. Total candidate search duration2.081s versus1.445s baseline.
-39 equal60ms single-thread pairs: the same2 decisions differ; aggregate iteration ratio0.714 (28.6% fewer iterations).
-39 equal350ms/12worker comparisons: **zero decision changes**, aggregate iteration ratio0.825 (17.5% fewer iterations). All78 arms completed. Total budgets27.30s, largest arm350.15ms. Both arms retain exactly the same root candidate masks and weights throughout all screens.

These results establish that scoring changes and its runtime cost are measurable. They do not show earlier escape or greater playing strength. The two more critical duel traps remain unchanged. The isolated350ms screen excludes tactical preparation but matches worker count and a representative foreground search interval.

**Recommendation: keep this evaluator implemented but opt-in pending controlled strength evidence.** The production-shaped comparison shows no move benefit and a17.5% throughput cost. Do not activate merely because a fixed-iteration seed changed direction. No parameter was tuned after looking at the results.

Parent review found an additional same-meal simultaneous-growth edge case to correct: a one-shorter rival tying on food also grants us growth; it must remain shorter. The snapshot's growth comparison can overstate such a gate. The parent will fix this and refresh affected artifacts. Do not treat this report's numbers as measurements of a later corrected implementation without a matching rerun/hash.

## Reproduce

```
RUSTC_WRAPPER= cargo run --release --offline --manifest-path experiments/enclosure-2026-10-15/audit/Cargo.toml -- audit
RUSTC_WRAPPER= cargo run --release --offline --manifest-path experiments/enclosure-2026-10-15/audit/Cargo.toml -- fixed
RUSTC_WRAPPER= cargo run --release --offline --manifest-path experiments/enclosure-2026-10-15/audit/Cargo.toml -- timed
RUSTC_WRAPPER= cargo run --release --offline --manifest-path experiments/enclosure-2026-10-15/audit/Cargo.toml -- parallel
```

106 library tests passed,1 existing ignored; Clippy `-D warnings` passed at subagent handoff. New tests cover a gated single exit, health/hazards/food and length-two neck legality, shorter rivals growing into contention, tail-route uncertainty, lethal own-hazard room exclusion, bounded replay scores and retained-child option propagation. Parent performs an independent review/fix pass and final combined test run.

## Parent review and final state

The parent reproduced the simultaneous-food-growth bug with a failing corner-exit fixture, then corrected the arrival comparison to grant both snakes growth when they tie on the target meal. A shorter rival therefore cannot gate that immediate exit solely because the shared meal grows it. The earlier-rival-growth test now places food before our arrival, preserving that distinct mechanism. First-step neck legality and health/hazard-feasible region access were also independently reviewed and covered by regressions.

The final production default is **off** for enclosure, both in `evaluate_board` and ordinary MCTS roots. Explicit `evaluate_board_with_options(..., true, true)` and `Node::new_root_with_enclosure_evaluation(board, true)` enable the experimental arm. A recorded board with a nonzero penalty protects the baseline default in tests; child expansion and retained snapshots preserve explicit settings. Food-race scoring remains active.

After the correction, all four screens were rerun without changing the weight or cases; use `final-audit.jsonl`, `final-fixed.jsonl`, `final-timed.jsonl`, `final-parallel.jsonl` and `final-summary.json` for the final implementation. All51 boards complete;22 get a penalty and17 a contested frontier. Mean assessment cost1.28µs locally.39 fixed-iteration pairs change2 decisions;39 single-thread60ms pairs change2 decisions. The39 production-shaped350ms/12-worker pairs change **zero decisions** and have median candidate/baseline throughput0.841 (**15.9% fewer iterations**). Root masks and weights are identical in every complete pair. No rescue or strength gain is established; opt-in is the justified activation choice.

Final combined verification:108 library unit tests pass, one existing diagnostic ignored;44 server tests pass; library/server Clippy across all targets with warnings denied and `git diff --check` pass. No commits or deployment.
