#!/usr/bin/env python3
"""Summarize paired-seat results and bootstrap a 95% interval over seeds."""

import json
import random
import sys
from collections import Counter
from pathlib import Path


def summarize(path, repetitions=20_000):
    rows = [json.loads(line) for line in Path(path).read_text().splitlines()]
    paired = {}
    for row in rows:
        paired.setdefault(row["seed"], {})[row["food_seat"]] = row
    if not paired or any(set(seats) != {0, 1} for seats in paired.values()):
        raise ValueError("expected complete two-seat pairs")

    per_seed = []
    outcomes = Counter(row["food_outcome"] for row in rows)
    for seed in sorted(paired):
        seats = paired[seed]
        points = sum(
            {"win": 1.0, "draw": 0.5, "loss": 0.0}[seats[seat]["food_outcome"]]
            for seat in (0, 1)
        ) / 2
        per_seed.append(points)

    score = sum(per_seed) / len(per_seed)
    rng = random.Random(91_773)
    samples = sorted(
        sum(rng.choices(per_seed, k=len(per_seed))) / len(per_seed)
        for _ in range(repetitions)
    )
    low = samples[int(0.025 * repetitions)]
    high = samples[min(repetitions - 1, int(0.975 * repetitions))]
    avg_turns = sum(row["turns"] for row in rows) / len(rows)
    print(f"file: {path}")
    print(f"seed pairs: {len(per_seed)}; games: {len(rows)}")
    print(f"food scorer score: {score:.3%}; paired seed-bootstrap 95% CI: [{low:.3%}, {high:.3%}]")
    print(f"wins/losses/draws: {outcomes['win']}/{outcomes['loss']}/{outcomes['draw']}")
    print(f"mean game turns: {avg_turns:.1f}")


if __name__ == "__main__":
    summarize(sys.argv[1])
