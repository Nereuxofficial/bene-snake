use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::{YouDeterminableGame, build_snake_id_map};
use battlesnake_game_types::wire_representation::Game as DEGame;
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use lib::mcts::Node;
use std::sync::Arc;

/// Helper function to create a node with expanded children
fn create_node_with_children(compact: CellBoard4Snakes11x11, num_expansions: usize) -> Arc<Node> {
    let you = compact.you_id().clone();
    let node = Arc::new(Node::new_root(compact));

    // Expand the node to create children
    for _ in 0..num_expansions {
        if node.is_fully_expanded() {
            break;
        }
        node.clone().expand(&you);
    }

    // Do some rollouts to populate win/visit statistics
    for _ in 0..10 {
        node.clone().rollout(&you);
    }

    node
}

/// Benchmark best_child with a start-of-game state
fn bench_best_child_start_of_game(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

    let node = create_node_with_children(compact, 4);

    c.bench_function("best_child_start_of_game", |b| {
        b.iter(|| {
            black_box(node.best_child(black_box(1.414)));
        })
    });
}

/// Benchmark best_child with a late stage game
fn bench_best_child_late_stage(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/late_stage.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

    let node = create_node_with_children(compact, 4);

    c.bench_function("best_child_late_stage", |b| {
        b.iter(|| {
            black_box(node.best_child(black_box(1.414)));
        })
    });
}

/// Benchmark best_child with a cornered scenario
fn bench_best_child_cornered(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/cornered.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

    let node = create_node_with_children(compact, 4);

    c.bench_function("best_child_cornered", |b| {
        b.iter(|| {
            black_box(node.best_child(black_box(1.414)));
        })
    });
}

/// Benchmark best_child with a 4-snake game
fn bench_best_child_4_snakes(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/4_snake_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

    let node = create_node_with_children(compact, 4);

    c.bench_function("best_child_4_snakes", |b| {
        b.iter(|| {
            black_box(node.best_child(black_box(1.414)));
        })
    });
}

/// Benchmark best_child with varying number of children
fn bench_best_child_varying_children(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

    let mut group = c.benchmark_group("best_child_varying_children");

    for num_children in [1, 2, 4, 8, 16].iter() {
        let node = create_node_with_children(compact.clone(), *num_children);

        group.bench_with_input(
            BenchmarkId::from_parameter(num_children),
            num_children,
            |b, _| {
                b.iter(|| {
                    black_box(node.best_child(black_box(1.414)));
                });
            },
        );
    }
    group.finish();
}

/// Benchmark best_child with different exploration constants
fn bench_best_child_varying_c(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

    let node = create_node_with_children(compact, 4);

    let mut group = c.benchmark_group("best_child_varying_c");

    for exploration_constant in [0.5, 1.0, 1.414, 2.0, 3.0].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{:.3}", exploration_constant)),
            exploration_constant,
            |b, &c_val| {
                b.iter(|| {
                    black_box(node.best_child(black_box(c_val)));
                });
            },
        );
    }
    group.finish();
}

/// Benchmark best_child across different scenarios
fn bench_best_child_scenarios(c: &mut Criterion) {
    let scenarios = vec![
        (
            "start_of_game",
            include_str!("../../battlesnake-game-types/fixtures/start_of_game.json"),
        ),
        (
            "late_stage",
            include_str!("../../battlesnake-game-types/fixtures/late_stage.json"),
        ),
        (
            "cornered",
            include_str!("../../battlesnake-game-types/fixtures/cornered.json"),
        ),
        (
            "4_snake_game",
            include_str!("../../battlesnake-game-types/fixtures/4_snake_game.json"),
        ),
        (
            "body_collision",
            include_str!("../../battlesnake-game-types/fixtures/body_collision.json"),
        ),
    ];

    let mut group = c.benchmark_group("best_child_scenarios");

    for (name, fixture) in scenarios {
        let g: Result<DEGame, _> = serde_json::from_slice(fixture.as_bytes());
        if let Ok(g) = g {
            let snake_id_mapping = build_snake_id_map(&g);
            if let Ok(compact) = g.as_cell_board(&snake_id_mapping) {
                let compact: CellBoard4Snakes11x11 = compact;
                let node = create_node_with_children(compact, 4);

                group.bench_with_input(BenchmarkId::from_parameter(name), &node, |b, node| {
                    b.iter(|| {
                        black_box(node.best_child(black_box(1.414)));
                    });
                });
            }
        }
    }
    group.finish();
}

/// Benchmark repeated best_child calls (simulating selection phase)
fn bench_best_child_repeated_calls(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

    let mut group = c.benchmark_group("best_child_repeated_calls");

    for num_calls in [10, 50, 100, 200].iter() {
        let node = create_node_with_children(compact.clone(), 4);

        group.bench_with_input(
            BenchmarkId::from_parameter(num_calls),
            num_calls,
            |b, &num_calls| {
                b.iter(|| {
                    for _ in 0..num_calls {
                        black_box(node.best_child(black_box(1.414)));
                    }
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_best_child_start_of_game,
    bench_best_child_late_stage,
    bench_best_child_cornered,
    bench_best_child_4_snakes,
    bench_best_child_varying_children,
    bench_best_child_varying_c,
    bench_best_child_scenarios,
    bench_best_child_repeated_calls,
);
criterion_main!(benches);
