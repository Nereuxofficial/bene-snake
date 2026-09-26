use std::{hint::black_box, sync::Arc, time::Duration};

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{YouDeterminableGame, build_snake_id_map},
    wire_representation::Game,
};
use criterion::{Criterion, criterion_group, criterion_main};
use lib::mcts::{Node, SearchDepthStats};

fn board(fixture: &str) -> CellBoard4Snakes11x11 {
    let game: Game = serde_json::from_str(fixture).expect("valid game fixture");
    let snake_ids = build_snake_id_map(&game);
    game.as_cell_board(&snake_ids).expect("compact board")
}

fn bench_rollout(c: &mut Criterion) {
    let mut group = c.benchmark_group("rollout");
    for (name, fixture) in [
        (
            "start_of_game",
            include_str!("../../battlesnake-game-types/fixtures/start_of_game.json"),
        ),
        (
            "late_stage",
            include_str!("../../battlesnake-game-types/fixtures/late_stage.json"),
        ),
    ] {
        let board = board(fixture);
        let you = *board.you_id();
        let node = Arc::new(Node::new_root(board));
        let mut stats = SearchDepthStats::default();
        group.bench_function(name, |b| {
            b.iter(|| black_box(Arc::clone(&node).rollout(black_box(&you), &mut stats)))
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(5));
    targets = bench_rollout
}
criterion_main!(benches);
