#!/usr/bin/env python3
"""Resumable paired-seat gym study of exact-state MCTS tree reuse."""

import argparse
import json
import os
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

STAGES = {
    "screen": (26_320_000, 32, 10),
    "confirm": (26_321_000, 128, 50),
    "overhead": (26_322_000, 16, 50),
}


def run_pair(binary, seed, budget, stage):
    result = subprocess.run(
        [str(binary), str(seed), str(budget), "150", stage],
        capture_output=True,
        text=True,
        check=True,
    )
    pair = [json.loads(line) for line in result.stdout.splitlines()]
    if len(pair) != 2 or {row["reuse_seat"] for row in pair} != {0, 1}:
        raise ValueError(f"invalid pair for seed {seed}: {result.stdout}")
    return pair


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("stage", choices=STAGES)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    first_seed, count, budget = STAGES[args.stage]
    binary = args.binary.resolve()
    output = Path(__file__).resolve().parent / f"{args.stage}.jsonl"
    rows = {}
    if output.exists():
        for line in output.read_text().splitlines():
            row = json.loads(line)
            key = (row["seed"], row["reuse_seat"])
            if key in rows:
                raise ValueError(f"duplicate {key}")
            rows[key] = row
    pending = []
    for seed in range(first_seed, first_seed + count):
        keys = [(seed, seat) for seat in (0, 1)]
        if all(key in rows for key in keys):
            continue
        if any(key in rows for key in keys):
            raise ValueError(f"partial pair {seed}")
        pending.append(seed)
    with output.open("a", encoding="utf-8") as stream, ThreadPoolExecutor(max_workers=4) as pool:
        for i, pair in enumerate(pool.map(lambda seed: run_pair(binary, seed, budget, args.stage), pending), 1):
            for row in pair:
                stream.write(json.dumps(row, sort_keys=True) + "\n")
            stream.flush()
            os.fsync(stream.fileno())
            if i % 8 == 0 or i == len(pending):
                print(f"{args.stage}: {i}/{len(pending)} seed pairs", flush=True)


if __name__ == "__main__":
    main()
