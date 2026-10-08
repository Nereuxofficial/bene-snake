# Certified decision-position gym

`snake-gym` compares production HTTP decisions on a frozen, exactly certified
synthetic corpus. Its score is accuracy on the declared scenario distribution.
It does not estimate Arena win rate or measure warm trees / between-turn pondering.

```sh
just gym compare --a ref:main --b ref:feature --suite screening-v1 --seed 1234
just gym compare --a bin:/tmp/bene-a --b bin:/tmp/bene-b --cases 60 --repeats 1 --seed 1234 --out gym-runs/check
just gym generate --suite balanced-v1 --cases 60 --seed 1234 --out /tmp/corpus
just gym verify-corpus /tmp/corpus
just gym compare --a ref:main --b bin:/tmp/bene-b --corpus /tmp/corpus --out gym-runs/check
just gym resume gym-runs/check
just gym report gym-runs/check
just gym serve gym-runs/check --bind 127.0.0.1:8050
just gym inspect gym-runs/check --case <case-id>
```

The default balanced profile accepts 240 independent base positions, with three
repeats and `game.timeout=500ms`. `screening-v1` uses 60 positions / one repeat;
`confirmation-v1` uses the balanced defaults and reserved constructor parameter
ranges (longer corridors, food/tail positions, isolated rival positions and
health margins outside the development ranges).
Confirmation has a separate corpus identity. Do not tune on its failures and
still describe it as held out. A suite JSON path can predeclare quotas, solver
budgets, inference minimums and practical thresholds. Counts must be positive
and divisible by the number of families: quotas and primary family weights are
equal. Unknown labels, duplicate states and quota shortages are explicit errors;
the generator never fills a shortage by weakening a proof.

The baseline constructors produce two-snake positions; the transition engine and
oracle also support three/four-snake coalitions. The six baseline families are unique escape, head contest, tail/growth, food access,
delayed folded-body corridor traps and forced sole survival. Horizons are 2–7
in the baseline profiles. They are constructive, synthetic examples with varied
health, lengths, paths, food distances, tail stacks and symmetry. They have no
claim of opening reachability or live frequency. The current terminal-win family
uses a constrained health deadline on the rival: it proves a terminal win but
is not a broad endgame distribution. Larger horizons do not imply harder cases.

## Challenge suite

Use `--suite hard-v1` for a separate, equal-weight challenge distribution:

- `crowded_duel`: two long, interwoven random-walk bodies and 7–8-turn survival.
- `coalition_escape`: three snakes with nearby heads and 4-turn coalition survival.
- `starvation_detour`: obstructed food routes with 6–8 health and a 7–9-turn survival objective.

The generator requires all four labels resolved, both Success and Failure, at
least two root moves that survive every first-turn opponent response, and a
Failure among those safe moves whose counterstrategy DAG has a delaying focal
continuation of at least four transitions. This continuation is not a minimum
death time or a claim about every opponent policy. The labels remain exact
robust finite-horizon guarantees. Random body walks only propose boards; the
candidate's choices never affect acceptance. Quota shortages and compute cutoffs
are explicit, and no trivial first-turn-only decision is accepted.

`hard-confirmation-v1` reserves longer horizons: crowded duel 9, coalition 5,
and starvation health 9–10 / horizon 10–11. Freeze confirmation before observing
candidate choices and do not tune the generator on its failures. Both challenge
compare presets default to 60 cases / three repeats; request timeout remains
500 ms. Pass `--cases` to choose the generated corpus size. Counts must divide by three.

```sh
just gym generate --suite hard-v1 --cases 60 --seed 20261020 --out /tmp/hard-corpus
just gym compare --a ref:main --b ref:feature --corpus /tmp/hard-corpus --repeats 3 --out gym-runs/hard-check
just gym compare --a ref:main --b ref:feature --suite hard-confirmation-v1 --seed 1234 --out gym-runs/hard-confirm
```

These densely packed synthetic bodies have no opening-reachability claim.
Compute-limited rejection also restricts the distribution to certifiable boards.
Keep the baseline suite for basic regressions; challenge scores are reported
separately. See `docs/gym-hard-suite.md` for calibration and measured performance.

Each case supports standard bounded 11×11 boards, 2–4 live snakes, no hazards and
**no future food** (`foodSpawnChance=minimumFood=0`). Initial food is consumed.
The reference transition engine follows movement, health, feeding and simultaneous
collisions; growth duplicates the new tail. Bodies move and disappear on death.
The oracle enumerates all cardinal opponent moves, including fatal/reverse moves,
as an adversarial coalition. It proves `survive(H)`, designated initial food plus
survival, or sole survival by H. Success is a robust finite guarantee. Failure
means an adversarial counterstrategy exists, not inevitable death under every
opponent policy. Node, wall and certificate cutoffs yield Unknown. The full DAG
verifier checks quantifiers, state transitions, progress and all four root labels.
The report's single continuation is an example from that DAG, not the proof.

Ref inputs resolve once to full commit SHAs. Git archives build in run-owned
source snapshots with separate target directories and `cargo build --release
--locked -p bene-snake`; dirty working-tree changes are excluded. The cache key
includes source hash, rustc output and flags. Source, lockfile, submodule state
and executable identities are recorded. Refs containing submodules currently
fail explicitly because archive mode cannot materialize them. Supplied binaries
are copied into run storage and never overwritten at the input path.

Every attempt uses a new process and unique game ID: readiness, `/start`, one
bounded `/move`, `/end`, process-group teardown. A and B run sequentially, in a
predeclared AB/BA schedule. The timeout covers sending the request through receipt
of the bounded full response body. Cleanup time never extends scoring time.
Expected labels/objectives are not sent to production candidates. Startup
configuration is preflighted; preflight failure invalidates the comparison.
After successful preflight, candidate failures score zero. Harness/I/O failure
leaves an incomplete block. Worker counts are determined by the target binary
and recorded as unobserved unless externally established.

Only PORT, an empty disabled GLITCHTIP_KEY, RUST_LOG and TZ are passed to candidate
processes; inherited configuration/credentials are scrubbed. An empty local runtime `.env` stops dotenv from searching ancestor directories. Current production servers bind `0.0.0.0`; the harness documents
that constraint and contacts them only over loopback. Run on a trusted host or
inside a network-isolated environment if that bind is unacceptable. The report
server itself rejects non-loopback binds and has read-only routes.

Run artifacts live under ignored `gym-runs/<run>/`: immutable manifest/hash,
frozen corpus, hashed certificates, append-only fsynced attempts, generation
costs/rejections, state, summary, bounded logs and `report/index.html`. Attempts
have unique scheduled keys. Resume validates manifest/settings, corpus,
certificates, binaries and the harness executable before measurement; it never
resolves moving branches again. A truncated final attempt line is recoverable;
earlier corruption is an error. The harness is also frozen as `binaries/snake-gym`; use that executable to
resume after a rebuild, since a different executable cannot resume the run.
Reports and inspection can still read artifacts with compatible rules/oracle
source hashes. Incomplete paired cases are excluded and timing gaps disclosed.

Scores average repetitions within cases and variants within base clusters, then
macro-average the declared families. The seeded 95% paired cluster bootstrap
resamples within each family (10,000 replicates); repeats do not increase
independent n. Inference requires complete coverage and at least five clusters
per family. The predeclared practical threshold is 2 percentage points. A verdict
requires the interval entirely beyond that threshold; otherwise it is
inconclusive, or inference unavailable. Five clusters is an availability floor,
not a statistical power guarantee.

Open `report/index.html` directly for offline search/filtering, per-repeat move
histograms, A/B arrows, all root labels and proof playback. Certificate downloads
are relative links into the saved run directory; retain that directory with the
HTML. JSON and names are escaped. No CDN, network fetch or server is needed for
the embedded report. `serve` is optional.

## Migration

Tournament/duel, gym opponent agents, the old full-game runner/statistics and
replay server are removed. Historical experiment crates importing `gym::agents`,
`GameConfig`, `run_game` or tournament statistics need their original Git
revision. There is no compatibility layer. Saved experiment files, binaries and
`gym-replays/` are preserved; the new tool does not consume or delete them.
Production snake strategy is unchanged by this migration.

## Validation

```sh
RUSTC_WRAPPER= cargo test -p gym
RUSTC_WRAPPER= cargo test --workspace
RUSTC_WRAPPER= cargo clippy -p gym --all-targets -- -D warnings
cargo fmt --all -- --check
just gym --help
```

HTTP tests need permission to bind loopback sockets and run supervised children.
The test-only Python fixture requires `/usr/bin/python3`; it is not used by the
production comparison runner. Corpus/calibration examples require no network.
See `docs/gym-redesign-acceptance.md` for measured release acceptance, example
reports and remaining limitations.
