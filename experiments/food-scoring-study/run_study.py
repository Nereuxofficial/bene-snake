#!/usr/bin/env python3
"""Resumable, paired-seat comparison of baseline and production food scoring."""

import argparse
import json
import os
import subprocess
from pathlib import Path

SCREEN_FIRST = 26_303_000
SCREEN_SEEDS = 64
CONFIRM_FIRST = 26_304_000
CONFIRM_SEEDS = 256


def load_rows(path):
    rows = {}
    if path.exists():
        for line in path.read_text().splitlines():
            row = json.loads(line)
            key = (row["seed"], row["food_seat"])
            if key in rows:
                raise ValueError(f"duplicate result {key} in {path}")
            rows[key] = row
    return rows


def run(binary, output, first_seed, seeds, budget_ms, pair_id):
    rows = load_rows(output)
    with output.open("a", encoding="utf-8") as stream:
        for offset in range(seeds):
            seed = first_seed + offset
            if all((seed, seat) in rows for seat in (0, 1)):
                continue
            if any((seed, seat) in rows for seat in (0, 1)):
                raise ValueError(f"incomplete seat pair for seed {seed}")
            result = subprocess.run(
                [str(binary), "1", str(budget_ms), str(seed), pair_id, "150"],
                text=True,
                capture_output=True,
                check=True,
            )
            pair = [json.loads(line) for line in result.stdout.splitlines()]
            if len(pair) != 2 or {row["food_seat"] for row in pair} != {0, 1}:
                raise ValueError(f"invalid seat pair for seed {seed}: {result.stdout}")
            for row in pair:
                stream.write(json.dumps(row, sort_keys=True) + "\n")
                rows[(seed, row["food_seat"])] = row
            stream.flush()
            os.fsync(stream.fileno())
            print(f"{pair_id}: {offset + 1}/{seeds} seeds", flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("stage", choices=("screen", "confirm"))
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    directory = Path(__file__).resolve().parent
    if args.stage == "screen":
        run(
            args.binary.resolve(),
            directory / "screen.jsonl",
            SCREEN_FIRST,
            SCREEN_SEEDS,
            10,
            "food-screen",
        )
    else:
        run(
            args.binary.resolve(),
            directory / "confirm.jsonl",
            CONFIRM_FIRST,
            CONFIRM_SEEDS,
            50,
            "food-confirm",
        )


if __name__ == "__main__":
    main()
