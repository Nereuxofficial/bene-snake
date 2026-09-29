#!/usr/bin/env python3
"""Summarize paired match score and actual search reuse."""

import json
import random
import sys
from collections import Counter, defaultdict
from pathlib import Path


def interval(values, reps=20_000):
    rng = random.Random(32127)
    samples = sorted(sum(rng.choices(values, k=len(values))) / len(values) for _ in range(reps))
    return samples[int(reps * .025)], samples[int(reps * .975)]


def main(paths):
    pairs = defaultdict(dict)
    for path in paths:
        for line in Path(path).read_text().splitlines():
            row = json.loads(line)
            seat = row["reuse_seat"]
            if seat in pairs[row["seed"]]:
                raise ValueError(f"duplicate seed/seat {row['seed']}/{seat}")
            pairs[row["seed"]][seat] = row
    if not pairs or any(set(seats) != {0, 1} for seats in pairs.values()):
        raise ValueError("expected complete two-seat pairs")
    rows = [row for seats in pairs.values() for row in seats.values()]
    scores = [sum({"win": 1, "draw": .5, "loss": 0}[row["reuse_outcome"]] for row in seats.values()) / 2 for seats in pairs.values()]
    lo, hi = interval(scores)
    outcomes = Counter(row["reuse_outcome"] for row in rows)
    total = lambda key: sum(row[key] for row in rows)
    calls = total("reuse_calls")
    fresh_calls = total("fresh_calls")
    print(f"pairs={len(pairs)} games={len(rows)} score={sum(scores)/len(scores):.3%} CI=[{lo:.3%},{hi:.3%}] W/L/D={outcomes['win']}/{outcomes['loss']}/{outcomes['draw']}")
    print(f"reuse hits={total('reuse_hits')}/{calls} ({total('reuse_hits')/calls:.2%}); carried visits={total('reuse_carried_visits')/calls:.1f}/move")
    print(f"new iterations/move: reuse={total('reuse_new_iterations')/calls:.1f} fresh={total('fresh_new_iterations')/fresh_calls:.1f}")
    print(f"effective visits/move: reuse={(total('reuse_carried_visits')+total('reuse_new_iterations'))/calls:.1f} fresh={total('fresh_new_iterations')/fresh_calls:.1f}")
    print(f"search wall ms/move: reuse={total('reuse_search_wall_us')/calls/1000:.2f} fresh={total('fresh_search_wall_us')/fresh_calls/1000:.2f}")
    if all("reuse_total_wall_us" in row for row in rows):
        print(f"whole call ms/move: reuse={total('reuse_total_wall_us')/calls/1000:.3f} fresh={total('fresh_total_wall_us')/fresh_calls/1000:.3f}")
        print(f"cache bookkeeping us/move: lookup={total('reuse_cache_lookup_us')/calls:.1f} prepare={total('reuse_cache_prepare_us')/calls:.1f}")


if __name__ == "__main__":
    main(sys.argv[1:])
