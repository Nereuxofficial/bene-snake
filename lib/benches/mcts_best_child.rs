use std::{hint::black_box, sync::Arc, time::Duration};

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{YouDeterminableGame, build_snake_id_map},
    wire_representation::Game,
};
use criterion::{Criterion, criterion_group, criterion_main};
use lib::mcts::Node;

fn root_with_visited_children(children: usize) -> Arc<Node> {
    let fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let game: Game = serde_json::from_str(fixture).expect("valid game fixture");
    let snake_ids = build_snake_id_map(&game);
    let board: CellBoard4Snakes11x11 = game.as_cell_board(&snake_ids).expect("compact board");
    let you = *board.you_id();
    let root = Arc::new(Node::new_root(board));

    for _ in 0..children {
        assert!(Arc::clone(&root).expand(&you), "enough legal actions");
    }
    // UCB1 prioritizes unvisited children. Visit each one so selection exercises scoring.
    for _ in 0..children {
        let (_, child) = root.best_child(1.6).expect("expanded child");
        child.backpropagate(500);
    }
    root
}

fn bench_best_child(c: &mut Criterion) {
    let mut group = c.benchmark_group("best_child");
    for children in [4, 16] {
        let root = root_with_visited_children(children);
        group.bench_function(format!("{children}_children"), |b| {
            b.iter(|| black_box(root.best_child(black_box(1.6))))
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(5));
    targets = bench_best_child
}
criterion_main!(benches);
