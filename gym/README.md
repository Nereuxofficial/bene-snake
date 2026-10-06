# Snake Gym

The gym pits the search agent in `lib::MctsAgent` against reference agents and
reports head-to-head and tournament results.

Run it from the repository root:

```sh
cargo run --release --package gym -- duel --agent1 mcts --agent2 heuristic --games 100 --seed 1234
cargo run --release --package gym -- tournament --agents mcts,heuristic,minimax,random --games 100
```

## Browser viewer and replays

Add `--web` to a duel, tournament, or benchmark to record games and serve a
clickable viewer at **http://127.0.0.1:8050**:

```sh
just gym --web duel --agent1 mcts --agent2 heuristic --games 20 --seed 1234
just gym --web tournament --agents mcts,heuristic,minimax,random --games 20 --parallel
```

The game list shows both running and finished games. Select a game to inspect
its board, food, snake health, and length. **Follow live** follows the latest turn
and advances to the next running game. Pause, step with the arrow buttons or
keyboard, scrub the turn slider, or change playback speed to inspect a replay.
Space toggles playback. Game links retain their selection in the URL; the
download link exports the complete replay as JSON.

The server stays open after the games finish; press **Ctrl-C** to stop it.
Completed games are saved atomically in `gym-replays/` (ignored by Git), and
appear again on subsequent runs. Interrupted games are not saved. You can
browse previously recorded games without running more:

```sh
just gym serve
just gym serve --web-bind 127.0.0.1:9000 --replay-dir /tmp/my-gym-replays
```

Use `--turn-delay 100` with `--web` to pause 100 ms after each published frame,
including turn zero, when fast agents finish too quickly to watch live.
Recording and serving add overhead; leave `--web` off for timing benchmarks.
Without `--web`, the gym neither records replays nor starts the web runtime.
Only games recorded with the viewer enabled are available in the history.
The viewer includes its assets locally and needs no frontend build or network
connection. The default bind address is loopback; `--web-bind 0.0.0.0:8050`
also makes it accessible from other machines on your network.

## Agent and policy variants

| CLI value | Agent | Notes |
|---|---|---|
| `mcts` | `lib::MctsAgent` | Not bit-for-bit reproducible: search rollouts use thread-local RNG. |
| `random` | `RandomAgent` | Uniform reasonable move from thread-local RNG. |
| `heuristic` | `HeuristicAgent` | **Default tactical policy.** |
| `heuristic-legacy` | `HeuristicAgent` | Pre-tactical policy, kept for A/B comparison. |
| `minimax` | `MinimaxAgent` | **Default paranoid policy.** |
| `minimax-legacy` | `MinimaxAgent` | Old joint-action search, kept for A/B comparison. |

### Heuristic

* `HeuristicPolicy::Tactical` (default) measures food distance, immediate free
  neighbors, and a capped reachable-area flood fill from the **candidate
  destination**, not the pre-move head. It disqualifies immediate starvation and
  treats a destination an equal/larger opponent can also enter as a head-on risk.
  It assumes no particular opponent move.
* `HeuristicPolicy::Legacy` is the original policy. It scored every move with the
  food distance from the current head, so it could not steer toward food, and it
  simulated a board where every opponent takes its first legal move.

Both policies are deterministic, so seeded duels between them are exactly
repeatable.

### Minimax

`MinimaxPolicy::Paranoid` (default) runs a depth-limited alpha-beta search where
each ply is one full turn: we choose a move, then the opponents jointly choose
theirs to minimize our value. Our own future moves are never adversarially
controlled, and terminal detection is based on `alive_snake_count`/`is_alive`,
so it is correct for any snake id and for multiplayer games.

`MinimaxPolicy::Legacy` alternates maximizing and minimizing over the cartesian
product of *every* snake's moves and uses the perspective-relative
`is_over`/`get_winner`. It is retained only so the search change can be measured.

The paranoid model is conservative: it assumes all opponents can coordinate
against us at once. That is not true simultaneous play, but it is a well-defined,
much stronger benchmark than the legacy search.

## Deterministic pairing

`duel` alternates seats by game index and reuses one seed for the paired swap, so
with an even `--games` count each starting position is played from both seats.
The winner is mapped back into user-facing agent order. An odd count leaves the
last seed unpaired and prints a note. Use an even count for balanced comparisons.

`HeadToHeadStats` reports both a win rate (draws count as non-wins) and a score
(draws count as half a point). `None` winners cover both mutual elimination and
turn-cap games with multiple survivors; both are draws.

## Validation results

All numbers below are from seeded duels on the standard 11x11, two-snake duel
board. Comparisons between deterministic agents (`heuristic`, `heuristic-legacy`,
`minimax`, `minimax-legacy`) are exactly repeatable; comparisons involving
`random` are not.

Tactical heuristic vs legacy heuristic (both seats per seed, 200 games each):

| Seeds | Tactical wins | Legacy wins | Draws |
|---|---:|---:|---:|
| 5,000,000 | 193 | 7 | 0 |
| 7,000,000 | 199 | 1 | 0 |

Paranoid minimax vs legacy minimax (`--max-turns 150`):

| Depth | Games | Paranoid W-L-D | Paranoid score | Legacy score |
|---|---:|---:|---:|---:|
| 2 | 200 | 94-27-79 | 66.8% | 33.2% |
| 3 | 60 | 15-10-35 | 54.2% | 45.8% |

The tactical heuristic scores 55.2% against paranoid minimax at depth 2 (48-27,
125 draws) but 75.8% against legacy minimax (111-8, 81 draws) on the same 200
seeds. The paranoid search is therefore a materially stronger opponent.

Against `random` on seed 8,000,000: the tactical heuristic wins 198-2 with no
draws, while the legacy heuristic scores 57.5% (98-68, 34 draws). The random
agent's own moves are unseeded, so these two rows can vary between runs.

## Known limitations

* MCTS games are not reproducible: `lib` search uses thread-local RNG, so
  "same seed" fixes the starting board and gym food RNG but not search outcomes.
  Fixed-seed deterministic comparisons currently rely on the heuristic/minimax.
* Draws are common, especially in four-snake games and between defensive minimax
  agents. Always read the score and the draw count, not only the win rate.
* The heuristic assumes no hazard damage. The gym never places hazards, so this
  is exact for gym games but would need revisiting for hazard rulesets.
* The heuristic's reachable-area flood fill treats single tails as free and
  otherwise ignores tail timing, so it over-approximates reachable space.
* Paranoid minimax coalitions all opponents and lets them respond after seeing
  our move. It is a benchmark opponent, not a claim about optimal multiplayer
  play.
* The gym's food spawning matches the compact simulator's minimum-1/15% rule.
  It is aligned with the default settings but is not configurable per ruleset.
