# Recognising unwinnable food races

## Problem and evidence

In replay `e6bcd8f8-8cf2-4474-93c7-2e773945b53f`, turn 276, bene-snake was length 20, health 46, and five Manhattan steps from food at (8,1). Length-21 Hovering Hobbs could also arrive in five steps. We approached for four more turns, then correctly avoided the immediate losing head contest. Hobbs ate it; we eventually starved. The earlier target choice spent time approaching a meal we could not safely claim by a direct equal-time contest.

That does not prove every possible plan lost that food race: obstacles, blocking, growth and different opponent choices can alter arrival. The goal is to recognise an adverse race early enough to choose another meal, while reserving “proved lost” for actual simulator evidence.

Before the leaf stage, the evaluator used nearest Manhattan food distance. The rollout policy still rewards progress toward nearest Manhattan food, and the root escape preference already has static path/rival arrival information. A new feature must improve target selection and commitment rather than duplicate generic flood fill.

## Assessment for each living snake and food

Return estimated arrival, path uncertainty, expected length at arrival, health margin, and a target classification with a reason. Distinguish bounds from estimates:

- Manhattan distance is an optimistic lower bound on travel, not a safe route.
- BFS over current occupancy provides a static route estimate. A missing static route does not prove unreachability because tails can move. A route through an optimistically released tail does not prove survival.
- For candidate routes, simulate initial steps with the production simulator so immediate body/neck legality, stacked tails, simultaneous growth and head contests match production.
- Track remaining health through the route, including hazards and food already encountered. In standard play, reaching food on the last available health step can restore health; validate this in the simulator rather than using a blanket strict-distance inequality.
- At equal arrival, compare lengths *at the contested turn*, including intervening growth. If a length estimate is uncertain, classify the race as contested/unknown. Do not assume present length persists over a long route.
- An opponent that can arrive earlier may eat the food before us; it does not have to wait there for a head collision. Conversely, a blocked or starving opponent should not automatically own food because of geometric proximity.

Do not predict future random food spawns as guaranteed resources.

## Classification

1. **Favourable estimate:** a feasible route appears to arrive before every rival, or wins the equal-arrival contest with sufficiently supported length estimates. This remains a preference unless a dynamic proof certifies it.
2. **Likely lost:** a rival has a feasible competing route arriving sooner, or at the same time with equal/greater length; our available route estimates give no credible way to beat it. Reduce attraction to this food *before* the heads become adjacent. Record which rival and which arrival comparison caused the assessment.
3. **Contested/unknown:** arrival intervals overlap, a route depends on tail release/growth, or work expires. Retain support for the food without treating it as secured or lost.
4. **Cannot reach before health expires:** even a valid optimistic lower bound exceeds the available health, under the explicitly modelled intervening food/hazard conditions. This is target infeasibility, not proof that the entire game is lost; other food can spawn or another route can pass an existing meal.

A true bounded “proved lost” diagnostic needs a **food-acquisition objective**, not just the existing survival objective. For a target food, enumerate simultaneous turns with our choices existential and opponent choices universal, ending in secured food only when we eat it and survive, target-lost when a rival eats it or we die, victory as its own terminal success, and unknown when horizon/work expires. Surviving while walking away from the food is not proof we can win the race. Keep this expensive diagnostic for near contests and offline certification initially.

## Decision policy

Score attainable meals by arrival margin, health remaining after travel, length advantage at arrival, and the escape route after eating. Penalise likely-lost races enough that a slightly farther attainable meal can beat a close opponent-owned one. Do not let growth reward or Manhattan proximity count an opponent-owned meal as expected food gain.

Choose a target, then recompute its assessment every real turn. Keep the target while its assessment remains competitive; switch immediately when a rival can take it first, the food disappears, the route closes, or a materially better health margin appears. Target persistence is a small preference, never a hard commitment. Use a short TTL and invalidate on game identity changes. Deeper search boards require fresh assessments; a root's old target/arrival estimates cannot become proofs on different boards.

If every target is adverse and health is running out, use a **best-chance** policy: retain reasonable moves and compare chances of reaching food/escape instead of eliminating every race and circling until starvation. Preserve immediate terminal wins and the existing proof-based survival rules. A generic distance penalty must not override a validated immediate win or create an empty move set.

## Implemented first stage

The root advisor is implemented in `lib/src/food_race.rs` and integrated with the root escape preferences. It replaces root food attraction with a single race-aware target, retains soft target continuity for three turns per game, and logs game/turn/request-correlated target diagnostics. It is enabled in the current working tree; it has not been deployed. `Node::new_root_with_food_races(board, false)` reproduces the previous root prior for controlled comparisons.

Work is bounded by 2 ms and 2,048 route expansions per root, with no additional simulator calls. Root masks and production move descriptions/occupancy semantics provide immediate legality; static route forecasts remain estimates. Simulator tests verify last-health-step eating and hazard/growth semantics. Exhaustion or absence of any static target retains the previous root policy. This first stage does not implement the goal-specific food-acquisition oracle or alter leaf evaluation, rollout/opponent policy, or rewards.

The replay audit covered 4,500 positions with zero mask changes and zero exhausted advisor budgets. Its maximum work was 617 expansions; local root preparation p99 was 0.029 ms. A four-seed, 20,000-iteration diagnostic on the seven recorded food-race boards confirms that MCTS can still overrule the changed prior, including turn 276. Root-only targeting therefore recognises the adverse race but does not establish starvation prevention. A separate leaf/rollout arm and equal-time full-game validation remain future work; no strength gain is claimed.

## Implemented leaf stage

The leaf stage now replaces nearest-Manhattan food attraction with the minimum route estimate plus bounded race penalties. It recomputes on simulated leaves, uses a deterministic work cap with legacy fallback, and preserves root masks, terminal rewards, food-gain rewards and rollout/opponent policies. Leaf options survive child creation and retained snapshots. `Node::new_root_with_food_race_evaluation(board, false)` isolates the old evaluator while retaining the root advisor.

Verification covered 679 positions from all three starvation games and 27 positions with four paired fixed-iteration seeds plus four equal-time repetitions per arm. At the critical turn 276, the leaf feature values the favourable ten-step alternative over the five-step adverse race, but the selected move remains right in every probe. Median equal-time throughput falls about 24 percent. This stage is implemented locally; starvation prevention and increased playing strength remain unproven. See `experiments/food-race-2026-10-15/LEAF-EVALUATION.md` for complete settings, results and limitations.

## Staged implementation

A. Add an offline root diagnostic over the saved replay corpus. Emit target, own/rival arrival estimates, rival length, health margin, confidence/reason, and route dependence. Extract e6bcd8f8 turns 275–281 and earlier food opportunities in the other two starvation games as fixtures. Label route estimates separately from exact bounded acquisitions.

B. First live experimental arm: use this assessment to replace the **root food-route prior's target selection**, retaining bounded support for all legal candidates. Suppress the directional attraction to a likely-lost target and prefer progress along a feasible route to the best alternative. No hard pruning, new food-gain reward, evaluator changes, or opponent-policy changes in this arm. Start with at most 2 ms and 2,048 simulator calls per root as provisional ceilings, charge all work to the existing move budget, and benchmark on the deployment CPU before selecting limits. BFS also needs a wall-time bound even when no simulator calls occur. Work exhaustion is unknown.

C. Measure whether the root-only change actually alters poor food approaches. If leaf Manhattan attraction overwhelms it, test a **separate** arm replacing the leaf food-distance term with a bounded race-adjusted term. Recompute/cache on the actual leaf/node board; do not reuse root distances. Measure total search throughput because path analysis at every rollout leaf can be expensive. Rollout-policy replacement is another separate arm, not bundled tuning.

## Acceptance and limitations

Targeted fixtures must cover:

- the e6bcd8f8 equal-arrival race against the longer snake, with the bad target recognised before immediate head collision;
- shorter opponent, equal lengths, and intervening food changing either snake's length;
- an earlier-arriving opponent consuming the target, a blocked rival, and a rival dying before arrival;
- a longer detour that is the only attainable meal, tail release, duplicate/stacked growth, and post-meal self-traps;
- exact last-health-step eating, a route exceeding health, hazards, and no current attainable food;
- unknown work budgets and immediate wins.

Track target quality, wasted approach turns, food acquired, starvation, head/body deaths, simulator work, and p95/p99 total request latency. On the recent corpus, rejecting a bad target is correctness evidence, not proof an alternative route wins the game.

Freeze baseline/candidate and settings, calibrate runtime, then run fresh equal-total-time games with all focal seats and complete paired seed blocks. Report uncertainty and strength/latency tradeoffs. Do not activate a food-race scoring change from the three observed starvation cases alone.
