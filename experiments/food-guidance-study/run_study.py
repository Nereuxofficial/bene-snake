#!/usr/bin/env python3
"""Resumable equal-budget, paired-seat food-guidance duel."""

import argparse
import json
import os
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

FIRST_SEED = 26_307_000
SEEDS = 768
BUDGET_MS = 50


def load_rows(path):
    rows = {}
    if path.exists():
        for line in path.read_text().splitlines():
            row = json.loads(line)
            key = (row["seed"], row["guided_seat"])
            if key in rows:
                raise ValueError(f"duplicate result {key} in {path}")
            rows[key] = row
    return rows


def run_pair(binary, seed):
    result = subprocess.run(
        [str(binary), "1", str(BUDGET_MS), str(seed), "food-guidance-confirm", "150"],
        text=True,
        capture_output=True,
        check=True,
    )
    pair = [json.loads(line) for line in result.stdout.splitlines()]
    if len(pair) != 2 or {row["guided_seat"] for row in pair} != {0, 1}:
        raise ValueError(f"invalid seat pair for seed {seed}: {result.stdout}")
    return pair


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    output = Path(__file__).resolve().parent / "confirm.jsonl"
    rows = load_rows(output)

    pending = []
    for offset in range(SEEDS):
        seed = FIRST_SEED + offset
        if all((seed, seat) in rows for seat in (0, 1)):
            continue
        if any((seed, seat) in rows for seat in (0, 1)):
            raise ValueError(f"incomplete seat pair for seed {seed}")
        pending.append(seed)

    with output.open("a", encoding="utf-8") as stream:
        with ThreadPoolExecutor(max_workers=4) as pool:
            for offset, (seed, pair) in enumerate(
                zip(pending, pool.map(lambda item: run_pair(binary, item), pending)),
                start=len(rows) // 2,
            ):
                for row in pair:
                    stream.write(json.dumps(row, sort_keys=True) + "\n")
                    rows[(seed, row["guided_seat"])] = row
                stream.flush()
                os.fsync(stream.fileno())
                print(f"confirm: {offset + 1}/{SEEDS} seeds", flush=True)


if __name__ == "__main__":
    main()
