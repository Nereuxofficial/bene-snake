use std::{
    alloc::System,
    hint::black_box,
    sync::Arc,
    time::{Duration, Instant},
};

use alloc_tracker::{Allocator, Session};
use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{Move, SimulableGame, SnakeId, YouDeterminableGame, build_snake_id_map},
    wire_representation::Game,
};
use criterion::{Criterion, criterion_group, criterion_main};
use lib::mcts::{Node, search_once};

#[global_allocator]
static ALLOCATOR: Allocator<System> = Allocator::system();

fn start_board() -> CellBoard4Snakes11x11 {
    let fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let game: Game = serde_json::from_str(fixture).expect("valid game fixture");
    let snake_ids = build_snake_id_map(&game);
    game.as_cell_board(&snake_ids).expect("compact board")
}

fn bench_rollout_allocations(c: &mut Criterion) {
    let board = start_board();
    let you = *board.you_id();
    let node = Arc::new(Node::new_root(board));
    let session = Session::new();
    let operation = session.operation("rollout");

    c.bench_function("allocations/rollout", |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            let span = operation.measure_thread().iterations(iters);
            for _ in 0..iters {
                black_box(Arc::clone(&node).rollout(black_box(&you)));
            }
            drop(span);
            start.elapsed()
        });
    });
}

fn bench_expand_allocations(c: &mut Criterion) {
    let board = start_board();
    let you = *board.you_id();
    let session = Session::new();
    let operation = session.operation("node_create_and_expand");

    c.bench_function("allocations/node_create_and_expand", |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            let span = operation.measure_thread().iterations(iters);
            for _ in 0..iters {
                let root = Arc::new(Node::new_root(board));
                search_once(&root, black_box(&you));
            }
            drop(span);
            start.elapsed()
        });
    });
}

fn bench_simulate_allocations(c: &mut Criterion) {
    let board = start_board();
    let moves = [
        (SnakeId(0), [Move::Up]),
        (SnakeId(1), [Move::Right]),
        (SnakeId(2), [Move::Down]),
        (SnakeId(3), [Move::Left]),
    ];
    let session = Session::new();
    let operation = session.operation("simulate_one_action");

    c.bench_function("allocations/simulate_one_action", |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            let span = operation.measure_thread().iterations(iters);
            for _ in 0..iters {
                black_box(board.simulate_with_moves(&moves).next());
            }
            drop(span);
            start.elapsed()
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(5));
    targets = bench_rollout_allocations, bench_expand_allocations, bench_simulate_allocations
}
criterion_main!(benches);
