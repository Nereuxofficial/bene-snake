use std::{hint::black_box, sync::Arc, time::Duration};

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{YouDeterminableGame, build_snake_id_map},
    wire_representation::Game,
};
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use lib::mcts::{Node, SearchDepthStats, search_once};

fn bench_expand(c: &mut Criterion) {
    let fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let game: Game = serde_json::from_str(fixture).expect("valid game fixture");
    let snake_ids = build_snake_id_map(&game);
    let board: CellBoard4Snakes11x11 = game.as_cell_board(&snake_ids).expect("compact board");
    let you = *board.you_id();

    c.bench_function("node_creation", |b| {
        b.iter(|| black_box(Node::new_root(black_box(board))))
    });
    c.bench_function("search_first_iteration", |b| {
        b.iter_batched(
            || Arc::new(Node::new_root(board)),
            |node| search_once(&node, black_box(&you), &mut SearchDepthStats::default()),
            BatchSize::SmallInput,
        )
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(5));
    targets = bench_expand
}
criterion_main!(benches);
