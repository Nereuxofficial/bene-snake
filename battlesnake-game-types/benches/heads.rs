//! Pins the cost of reading heads and of comparing head positions.
//!
//! Collected heads in an `ArrayVec` were measurably slower (about 6ns versus 2ns for a 4-snake
//! board) because `ArrayVec::push` is panic-capable, so `all_heads` hands back an iterator.
//! `iter_living_heads` filters `healths` and then indexes `heads`, rather than zipping the two
//! arrays: the zip advanced the head cursor for dead snakes too and cost 6-37% on this group.
//!
//! `other_head_within` decomposes the query cell once rather than per head, which measured 27-33%
//! faster than calling `cells_are_within` per head. `has_unresolved_tactical_conflict` costs about
//! 35ns for all four snakes on an opening board (49ns when its destinations were collected into
//! `ArrayVec`s first, the same penalty as `all_heads`).
//!
//! The `head_proximity_*` groups all answer the same question over every pair of living heads:
//! building `Position`s is the baseline, `cell_distance` is the general Manhattan test, and
//! `cells_are_adjacent` is the division-free one-step test. Per pair that is about 1.7ns, 1.4ns
//! and 0.8ns respectively, so hot paths that only need "next move" reach should use adjacency.

use battlesnake_game_types::{
    compact_representation::{
        CellIndex,
        standard::{CONTEST_RADIUS, CellBoard4Snakes11x11},
    },
    types::{HeadGettableGame, HealthGettableGame, SnakeId, build_snake_id_map},
    wire_representation::Game,
};
use criterion::{Criterion, criterion_group, criterion_main};
use std::{hint::black_box, time::Duration};

fn boards() -> Vec<(&'static str, CellBoard4Snakes11x11)> {
    [
        (
            "start_of_game",
            include_str!("../fixtures/start_of_game.json"),
        ),
        ("late_stage", include_str!("../fixtures/late_stage.json")),
        ("tail_chase", include_str!("../fixtures/tail_chase.json")),
        (
            "four_snake_game",
            include_str!("../fixtures/4_snake_game.json"),
        ),
    ]
    .into_iter()
    .map(|(name, text)| {
        let game: Game = serde_json::from_str(text).unwrap();
        let ids = build_snake_id_map(&game);
        let board = game.as_cell_board(&ids).unwrap();
        (name, board)
    })
    .collect()
}

fn bench_heads(c: &mut Criterion) {
    let fixtures = boards();
    let mut fast = c.benchmark_group("all_heads");
    for (name, board) in &fixtures {
        // One pass over the head array.
        fast.bench_function(*name, |b| {
            b.iter(|| {
                let mut total = 0usize;
                for head in black_box(board).all_heads() {
                    total += head.as_usize();
                }
                black_box(total)
            })
        });
    }
    fast.finish();
    let mut slow = c.benchmark_group("all_heads_per_snake");
    for (name, board) in &fixtures {
        // Asking for one id at a time, which the compiler fully unrolls.
        slow.bench_function(*name, |b| {
            b.iter(|| {
                let mut total = 0usize;
                for index in 0..4u8 {
                    let id = SnakeId(index);
                    if black_box(board).get_health_i64(&id) > 0 {
                        total += black_box(board).get_head_as_native_position(&id).as_usize();
                    }
                }
                black_box(total)
            })
        });
    }
    slow.finish();
    // Proximity between two heads, over every pair of living heads. `head_proximity_positions`
    // is what a caller writes without index helpers: build Positions, then i32 arithmetic. The
    // heads are collected outside the timed closure so the groups differ only in the test itself.
    let mut proximity_positions = c.benchmark_group("head_proximity_positions");
    for (name, board) in &fixtures {
        let heads: Vec<(SnakeId, CellIndex<u8>)> = board.all_heads_with_ids().collect();
        proximity_positions.bench_function(*name, |b| {
            b.iter(|| {
                let board = black_box(board);
                let mut contested = false;
                for (index, (id, _)) in heads.iter().enumerate() {
                    let ours = board.get_head_as_position(id);
                    for (other_id, _) in heads.iter().skip(index + 1) {
                        let theirs = board.get_head_as_position(other_id);
                        let dx = ours.x - theirs.x;
                        let dy = ours.y - theirs.y;
                        if dx.abs() + dy.abs() == 1 {
                            contested = true;
                        }
                    }
                }
                black_box(contested)
            })
        });
    }
    proximity_positions.finish();
    let mut proximity_distance = c.benchmark_group("head_proximity_cell_distance");
    for (name, board) in &fixtures {
        let heads: Vec<(SnakeId, CellIndex<u8>)> = board.all_heads_with_ids().collect();
        proximity_distance.bench_function(*name, |b| {
            b.iter(|| {
                let board = black_box(board);
                let mut contested = false;
                for (index, (_, head)) in heads.iter().enumerate() {
                    for (_, other) in heads.iter().skip(index + 1) {
                        if board.cell_distance(*head, *other) == 1 {
                            contested = true;
                        }
                    }
                }
                black_box(contested)
            })
        });
    }
    proximity_distance.finish();
    let mut proximity_adjacent = c.benchmark_group("head_proximity_adjacent");
    for (name, board) in &fixtures {
        let heads: Vec<(SnakeId, CellIndex<u8>)> = board.all_heads_with_ids().collect();
        proximity_adjacent.bench_function(*name, |b| {
            b.iter(|| {
                let board = black_box(board);
                let mut contested = false;
                for (index, (_, head)) in heads.iter().enumerate() {
                    for (_, other) in heads.iter().skip(index + 1) {
                        if board.cells_are_adjacent(*head, *other) {
                            contested = true;
                        }
                    }
                }
                black_box(contested)
            })
        });
    }
    proximity_adjacent.finish();
    let mut scan = c.benchmark_group("other_head_within");
    for (name, board) in &fixtures {
        let heads: Vec<(SnakeId, CellIndex<u8>)> = board.all_heads_with_ids().collect();
        scan.bench_function(*name, |b| {
            b.iter(|| {
                let board = black_box(board);
                let mut found = false;
                for (id, head) in &heads {
                    found |= board.other_head_within(*id, *head, CONTEST_RADIUS);
                }
                black_box(found)
            })
        });
    }
    scan.finish();
    let mut scan = c.benchmark_group("other_head_within_radius_one");
    for (name, board) in &fixtures {
        let heads: Vec<(SnakeId, CellIndex<u8>)> = board.all_heads_with_ids().collect();
        scan.bench_function(*name, |b| {
            b.iter(|| {
                let board = black_box(board);
                let mut found = false;
                for (id, head) in &heads {
                    found |= board.other_head_within(*id, *head, 1);
                }
                black_box(found)
            })
        });
    }
    scan.finish();
    let mut conflict = c.benchmark_group("has_unresolved_tactical_conflict");
    for (name, board) in &fixtures {
        conflict.bench_function(*name, |b| {
            b.iter(|| {
                let board = black_box(board);
                let mut count = 0usize;
                for (id, _) in board.all_heads_with_ids() {
                    count += usize::from(board.has_unresolved_tactical_conflict(id));
                }
                black_box(count)
            })
        });
    }
    conflict.finish();
    let mut masks = c.benchmark_group("reasonable_move_masks");
    for (name, board) in &fixtures {
        masks.bench_function(*name, |b| {
            b.iter(|| {
                let mut total = 0u8;
                for (_, mask) in black_box(board).reasonable_move_masks() {
                    total |= mask;
                }
                black_box(total)
            })
        });
    }
    masks.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(3));
    targets = bench_heads
}
criterion_main!(benches);
