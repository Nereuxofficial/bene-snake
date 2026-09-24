use std::{hint::black_box, sync::Arc, time::Duration};

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{YouDeterminableGame, build_snake_id_map},
    wire_representation::Game,
};
use criterion::{Criterion, criterion_group, criterion_main};
use lib::mcts::{Node, search_once};

fn root_after_iterations(iterations: usize) -> Arc<Node> {
    let fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let game: Game = serde_json::from_str(fixture).expect("valid game fixture");
    let snake_ids = build_snake_id_map(&game);
    let board: CellBoard4Snakes11x11 = game.as_cell_board(&snake_ids).expect("compact board");
    let you = *board.you_id();
    let root = Arc::new(Node::new_root(board));

    for _ in 0..iterations {
        search_once(&root, &you);
    }
    root
}

fn bench_best_move(c: &mut Criterion) {
    let mut group = c.benchmark_group("best_move");
    for iterations in [4, 16] {
        let root = root_after_iterations(iterations);
        let you = battlesnake_game_types::types::SnakeId(0);
        group.bench_function(format!("{iterations}_iterations"), |b| {
            b.iter(|| black_box(root.best_move(black_box(you))))
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(5));
    targets = bench_best_move
}
criterion_main!(benches);
