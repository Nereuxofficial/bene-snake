# MCTS hot-loop benchmarks

`just bench` runs the timing suite. `just bench-alloc` runs allocation tracking separately. Direct Cargo invocations require `--features bench`; benchmark-only accessors expose the existing private kernels without changing production behavior. `just profile-mcts-rollout` profiles the production RNG on the crowded four-snake fixture. `just profile-bench lib mcts_rollout 60 rollout_seeded/duel_dense` selects another fixture; the recipe enables the feature and obtains the current executable from Cargo's artifact report. Omitting the filter profiles every matching benchmark for the requested duration each.

The shared fixtures are seven captured Arena requests, not synthetic all-Up actions. Setup parses each request, builds the ID map, collects its food, and loads 64 frozen policy-sampled joint actions before measurement. Actions are remapped through captured wire snake IDs, so ID assignment changes do not change their meaning. Freezing the action inputs also prevents future policy changes from silently changing isolated simulation work. All captures have a living own snake and at least two living snakes. This is a small fixed workload suite, not a statistical sample of every Arena situation.

| Fixture | Game / turn | Snakes | Own length / health | Food |
|---|---|---:|---:|---:|
| four_opening | `6a780278-fac1-4961-91ef-534590ec959a` / 0 | 4 | 3 / 100 | 5 |
| four_crowded | `0928cd9b-733f-42dd-8f79-42a0756d8f04` / 50 | 4 | 13 / 100 | 1 |
| three_midgame | `6a780278-fac1-4961-91ef-534590ec959a` / 50 | 3 | 7 / 97 | 1 |
| duel_food_race | `6a780278-fac1-4961-91ef-534590ec959a` / 82 | 2 | 10 / 92 | 2 |
| duel_dense | `6a780278-fac1-4961-91ef-534590ec959a` / 248 | 2 | 23 / 81 | 10 |
| duel_late | `6a780278-fac1-4961-91ef-534590ec959a` / 285 | 2 | 23 / 44 | 7 |
| three_hungry | `17fc9b05-b09a-4ac2-8cc7-55f7301d9978` / 161 | 3 | 11 / 24 | 2 |

## Timing scopes

- `rollout_seeded`: existing rollout kernel, caller-owned fixed-seed `SmallRng`, fixed root board. RNG initialization, parsing, and root construction are excluded. One element means one whole rollout, not one simulated turn.
- `rollout_production_rng`: the same rollout kernel with the production `ThreadRng`; real sampling cost, but sampled trajectories vary between runs. Use the seeded lane for controlled algorithm comparisons.
- `root_prepare`: cold move-cache initialization plus actual root escape/food guidance. Root construction and destruction are excluded. Actual cached policy weights are used.
- `search_fixed_iterations/cold`: the first search iteration on an unprepared root.
- `search_fixed_iterations/growing`: 512 iterations from a fresh root, including initial root preparation and cache growth.
- `search_fixed_iterations/decision`: 16,384 iterations from a fresh root, to expose deeper traversal and larger tree working sets over a complete fixed-work decision.
- `search_fixed_iterations/warm`: exactly 512 iterations after exactly 2,048 warmup iterations. A fresh seeded root is rebuilt for every sample. Warmup and tree destruction are excluded, so Criterion does not gradually turn this into a different tree size or terminal-leaf workload.
- `rollout_policy_seeded`: actual reasonable masks, snake facts, weights, and joint-action sampling with supplied food. It includes policy RNG draws, excludes simulation and food scanning.
- `legal_move_masks`: all living snakes' masks for one fixed board.
- `candidate_neighbor_count`: mobility counts at all reasonable candidate destinations, as used by policy weighting; destination generation is outside the measured block.
- `simulate_single_action_batch`: 64 sampled joint actions, each applied independently to the same board. It uses the direct simulator and excludes action generation. This measures per-turn simulation across varied actions; sustained evolving trajectories are measured by full rollouts/search.
- `leaf_evaluate_supplied_food`: production leaf evaluation with the food list already available, as in rollout. `food_positions` additionally compares scanning the board with supplied-food evaluation.
- `best_move_warm_tree`: final move selection after 2,048 seeded iterations, using the fixture's mapped own snake ID.

Search batches call the same `search_iteration` kernel as production, with preparation once per batch and the same policy, widening, rollout, and backup behavior. They use `SmallRng` to repeat work rather than an elapsed-time stop. Thread creation, deadline/cancellation checks, API normalization, cross-turn tree reuse, result publication, telemetry, and cleanup are outside this kernel suite. Equal-time gym tests remain necessary to assess playing strength.

## Allocation scopes

The allocation suite uses the same fixtures and seeds:

- Seeded rollout: root construction and RNG initialization excluded.
- Cold first iteration: includes root allocation, cold preparation, expansion, and cleanup, matching the existing root-lifecycle allocation metric.
- Warm iterations: warmup/root construction/destruction excluded; counts divided by the 512 actual iterations, not by the number of batches.
- Direct simulation: counts divided by the 64 actual actions per batch.

Allocation tracking changes timing; use it for allocation counts and the separate uninstrumented suite for throughput. Captured food is static within isolated kernels; full rollouts preserve the production simulator's consumption behavior and do not spawn replacement food.

## Commands

```sh
# Full suite; the broader fixture/phase coverage takes several minutes.
RUSTC_WRAPPER= just bench
RUSTC_WRAPPER= just bench-alloc

# Quick validation / shortened measurements.
RUSTC_WRAPPER= just bench --test
RUSTC_WRAPPER= just bench search_fixed_iterations/warm --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2
RUSTC_WRAPPER= just bench-alloc allocations/simulate_single_action_batch --test

# Optional full-scan versus supplied-food comparison.
RUSTC_WRAPPER= cargo bench -p lib --features bench --bench food_positions

# Check fixture coverage, direct/general parity and repeatable search samples.
RUSTC_WRAPPER= cargo test -p lib --features bench --test bench_workloads
```

Metric names changed deliberately. Establish new before/after baselines using identical fixtures, seeds, build flags and batch sizes; do not compare these results directly with the old generic opening/late-stage numbers. Shortened timing runs verify the suite works and are not evidence of a speed improvement. The standalone `mcts_pruning` and general all-joint-action simulator probes remain supplementary workloads, outside `just bench`.

To intentionally regenerate the frozen simulator actions with the current policy, run `RUSTC_WRAPPER= cargo run --release -p lib --features bench --example bench_fixture_actions` from the repository root. This changes benchmark inputs and requires new baselines.
