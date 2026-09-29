#!/usr/bin/env python3
"""Report paired win-score uncertainty and measured food behavior."""

import json
import random
import sys
from collections import Counter, defaultdict
from pathlib import Path


def interval(values, reps=20_000):
    rng = random.Random(91029)
    means = sorted(sum(rng.choices(values, k=len(values))) / len(values) for _ in range(reps))
    return means[int(reps * .025)], means[int(reps * .975)]


def summarize(path):
    groups = defaultdict(lambda: defaultdict(dict))
    for line in Path(path).read_text().splitlines():
        row = json.loads(line)
        key = (row["candidate_weight"], row["candidate_length_weight"])
        seat = row["candidate_seat"]
        if seat in groups[key][row["seed"]]:
            raise ValueError(f"duplicate result {key} {row['seed']} seat {seat}")
        groups[key][row["seed"]][seat] = row
    for (food, length), seeds in sorted(groups.items()):
        if any(set(pair) != {0, 1} for pair in seeds.values()):
            raise ValueError(f"incomplete pairs for {food},{length}")
        pairs = [list(seeds[seed].values()) for seed in sorted(seeds)]
        rows = [row for pair in pairs for row in pair]
        scores = [sum({"win": 1, "draw": .5, "loss": 0}[row["candidate_outcome"]] for row in pair) / 2 for pair in pairs]
        lo, hi = interval(scores)
        food_diffs = [sum(row["candidate_food_eaten"] - row["baseline_food_eaten"] for row in pair) / 2 for pair in pairs]
        food_lo, food_hi = interval(food_diffs)
        early_diffs = [sum(row["candidate_early_food_eaten"] - row["baseline_early_food_eaten"] for row in pair) / 2 for pair in pairs]
        early_lo, early_hi = interval(early_diffs)
        toward_diffs = [
            (sum(row["candidate_moves_closer_to_food"] for row in pair) / sum(row["candidate_moves"] for row in pair))
            - (sum(row["baseline_moves_closer_to_food"] for row in pair) / sum(row["baseline_moves"] for row in pair))
            for pair in pairs
        ]
        toward_lo, toward_hi = interval(toward_diffs)
        outcome = Counter(row["candidate_outcome"] for row in rows)
        def total(prefix, name):
            return sum(row[f"{prefix}_{name}"] for row in rows)
        cm = total("candidate", "moves")
        bm = total("baseline", "moves")
        print(
            f"food={food} length={length} seeds={len(seeds)} games={len(rows)} "
            f"score={sum(scores)/len(scores):.3%} CI=[{lo:.3%},{hi:.3%}] "
            f"W/L/D={outcome['win']}/{outcome['loss']}/{outcome['draw']} "
            f"toward={total('candidate','moves_closer_to_food')/cm:.3%}/{total('baseline','moves_closer_to_food')/bm:.3%} "
            f"food={total('candidate','food_eaten')}/{total('baseline','food_eaten')} "
            f"food_per_game={total('candidate','food_eaten')/len(rows):.2f}/{total('baseline','food_eaten')/len(rows):.2f} "
            f"food_diff_CI=[{food_lo:.2f},{food_hi:.2f}] "
            f"early_food={total('candidate','early_food_eaten')}/{total('baseline','early_food_eaten')} "
            f"early_diff_CI=[{early_lo:.2f},{early_hi:.2f}] "
            f"toward_diff_CI=[{toward_lo:.3%},{toward_hi:.3%}] "
            f"onto_food={total('candidate','moves_onto_food')}/{total('baseline','moves_onto_food')} "
            f"moves={cm}/{bm}"
        )


if __name__ == "__main__":
    summarize(sys.argv[1])
