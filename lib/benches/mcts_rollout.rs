use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::{YouDeterminableGame, build_snake_id_map};
use battlesnake_game_types::wire_representation::Game as DEGame;
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use lib::mcts::Node;
use std::sync::Arc;

/// Benchmark the rollout function with different game states
fn bench_rollout_start_of_game(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = compact.you_id().clone();
    let node = Arc::new(Node::new_root(compact, &you));

    c.bench_function("rollout_start_of_game", |b| {
        b.iter(|| {
            black_box(node.clone().rollout(black_box(&you)));
        })
    });
}

/// Benchmark rollout with a late stage game (fewer snakes, more space)
fn bench_rollout_late_stage(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/late_stage.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = compact.you_id().clone();
    let node = Arc::new(Node::new_root(compact, &you));

    c.bench_function("rollout_late_stage", |b| {
        b.iter(|| {
            black_box(node.clone().rollout(black_box(&you)));
        })
    });
}

/// Benchmark rollout with a cornered scenario (limited moves)
fn bench_rollout_cornered(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/cornered.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = compact.you_id().clone();
    let node = Arc::new(Node::new_root(compact, &you));

    c.bench_function("rollout_cornered", |b| {
        b.iter(|| {
            black_box(node.clone().rollout(black_box(&you)));
        })
    });
}

/// Benchmark rollout with a 4-snake game
fn bench_rollout_4_snakes(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/4_snake_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = compact.you_id().clone();
    let node = Arc::new(Node::new_root(compact, &you));

    c.bench_function("rollout_4_snakes", |b| {
        b.iter(|| {
            black_box(node.clone().rollout(black_box(&you)));
        })
    });
}

/// Benchmark rollout multiple times to measure variance
fn bench_rollout_multiple_runs(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = compact.you_id().clone();

    let mut group = c.benchmark_group("rollout_multiple_runs");

    for num_rollouts in [1, 5, 10, 20, 50].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_rollouts),
            num_rollouts,
            |b, &num_rollouts| {
                let node = Arc::new(Node::new_root(compact.clone(), &you));
                b.iter(|| {
                    for _ in 0..num_rollouts {
                        black_box(node.clone().rollout(black_box(&you)));
                    }
                });
            },
        );
    }
    group.finish();
}

/// Benchmark different scenarios in a parameterized way
fn bench_rollout_scenarios(c: &mut Criterion) {
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

    let mut group = c.benchmark_group("rollout_scenarios");

    for (name, fixture) in scenarios {
        let g: Result<DEGame, _> = serde_json::from_slice(fixture.as_bytes());
        if let Ok(g) = g {
            let snake_id_mapping = build_snake_id_map(&g);
            if let Ok(compact) = g.as_cell_board(&snake_id_mapping) {
                let compact: CellBoard4Snakes11x11 = compact;
                let you = compact.you_id().clone();
                let node = Arc::new(Node::new_root(compact, &you));

                group.bench_with_input(BenchmarkId::from_parameter(name), &node, |b, node| {
                    b.iter(|| {
                        black_box(node.clone().rollout(black_box(&you)));
                    });
                });
            }
        }
    }
    group.finish();
}

/// Benchmark the full MCTS search with limited iterations
fn bench_mcts_search_limited(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = compact.you_id().clone();

    let mut group = c.benchmark_group("mcts_search_limited");
    group.sample_size(10); // Reduce sample size for longer-running benchmarks

    // Note: This would require exposing an iteration parameter in mcts_search
    // For now, we'll just benchmark the rollout which is the core component
    let node = Arc::new(Node::new_root(compact, &you));

    group.bench_function("50_rollouts", |b| {
        b.iter(|| {
            for _ in 0..50 {
                black_box(node.clone().rollout(black_box(&you)));
            }
        });
    });

    group.finish();
}

/// Benchmark node creation overhead
fn bench_node_creation(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = compact.you_id().clone();

    c.bench_function("node_creation", |b| {
        b.iter(|| {
            black_box(Node::new_root(black_box(compact.clone()), black_box(&you)));
        })
    });
}

criterion_group!(
    benches,
    bench_rollout_start_of_game,
    bench_rollout_late_stage,
    bench_rollout_cornered,
    bench_rollout_4_snakes,
    bench_rollout_multiple_runs,
    bench_rollout_scenarios,
    bench_mcts_search_limited,
    bench_node_creation,
);
criterion_main!(benches);
