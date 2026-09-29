#!/usr/bin/env python3
"""Resumable seed-paired gym comparison of food-gain rewards."""

import argparse
import json
import os
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

CONFIGS = {
    "screen": (26_312_000, 32, 10, [20, 50, 100]),
    "confirm": (26_313_000, 128, 50, []),
    "extended": (26_314_000, 384, 50, []),
}


def run_pair(binary, seed, budget, reward, stage):
    result = subprocess.run(
        [str(binary), str(seed), str(budget), str(reward), "150", stage],
        capture_output=True,
        text=True,
        check=True,
    )
    pair = [json.loads(line) for line in result.stdout.splitlines()]
    if len(pair) != 2 or {row["candidate_seat"] for row in pair} != {0, 1}:
        raise ValueError(f"invalid pair for seed {seed}: {result.stdout}")
    return pair


def run_stage(binary, stage, configs):
    first_seed, count, budget, _ = CONFIGS[stage]
    output = Path(__file__).resolve().parent / f"{stage}.jsonl"
    rows = {}
    if output.exists():
        for line in output.read_text().splitlines():
            row = json.loads(line)
            key = (row["candidate_reward"], row["seed"], row["candidate_seat"])
            if key in rows:
                raise ValueError(f"duplicate {key}")
            rows[key] = row
    tasks = []
    for reward in configs:
        for seed in range(first_seed, first_seed + count):
            pair_keys = [(reward, seed, seat) for seat in (0, 1)]
            if all(key in rows for key in pair_keys):
                continue
            if any(key in rows for key in pair_keys):
                raise ValueError(f"partial pair {pair_keys}")
            tasks.append((seed, budget, reward, stage))
    with output.open("a", encoding="utf-8") as stream, ThreadPoolExecutor(max_workers=4) as pool:
        for i, pair in enumerate(pool.map(lambda args: run_pair(binary, *args), tasks), 1):
            for row in pair:
                stream.write(json.dumps(row, sort_keys=True) + "\n")
            stream.flush()
            os.fsync(stream.fileno())
            if i % 8 == 0 or i == len(tasks):
                print(f"{stage}: {i}/{len(tasks)} seed pairs", flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("stage", choices=CONFIGS)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--candidate", action="append", type=int, help="food gain reward; required for confirm")
    args = parser.parse_args()
    if args.stage in ("confirm", "extended"):
        if not args.candidate:
            parser.error(f"{args.stage} requires --candidate REWARD")
        configs = args.candidate
    else:
        configs = CONFIGS["screen"][3]
    run_stage(args.binary.resolve(), args.stage, configs)


if __name__ == "__main__":
    main()
