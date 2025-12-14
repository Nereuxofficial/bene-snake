use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::{YouDeterminableGame, build_snake_id_map};
use battlesnake_game_types::wire_representation::Game as DEGame;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use lib::mcts::Node;
use std::hint::black_box;
use std::sync::Arc;

/// Benchmark single expand call on a fresh node
fn bench_expand_single(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = *compact.you_id();

    c.bench_function("expand_single", |b| {
        b.iter_batched(
            || Arc::new(Node::new_root(compact)),
            |node| {
                black_box(node.expand(black_box(&you)));
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

/// Benchmark expanding a node multiple times until fully expanded
fn bench_expand_until_full(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = *compact.you_id();

    c.bench_function("expand_until_full", |b| {
        b.iter_batched(
            || Arc::new(Node::new_root(compact)),
            |node| {
                while !node.is_fully_expanded() {
                    black_box(node.clone().expand(black_box(&you)));
                }
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

/// Benchmark expand with different game states
fn bench_expand_scenarios(c: &mut Criterion) {
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

    let mut group = c.benchmark_group("expand_scenarios");

    for (name, fixture) in scenarios {
        let g: Result<DEGame, _> = serde_json::from_slice(fixture.as_bytes());
        if let Ok(g) = g {
            let snake_id_mapping = build_snake_id_map(&g);
            if let Ok(compact) = g.as_cell_board(&snake_id_mapping) {
                let compact: CellBoard4Snakes11x11 = compact;
                let you = *compact.you_id();

                group.bench_with_input(
                    BenchmarkId::from_parameter(name),
                    &compact,
                    |b, compact| {
                        b.iter_batched(
                            || Arc::new(Node::new_root(*compact)),
                            |node| {
                                black_box(node.expand(black_box(&you)));
                            },
                            criterion::BatchSize::SmallInput,
                        );
                    },
                );
            }
        }
    }
    group.finish();
}

/// Benchmark expand with varying numbers of expansions
fn bench_expand_n_times(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = *compact.you_id();

    let mut group = c.benchmark_group("expand_n_times");

    for num_expansions in [1, 5, 10, 20, 50, 100].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_expansions),
            num_expansions,
            |b, &num_expansions| {
                b.iter_batched(
                    || Arc::new(Node::new_root(compact)),
                    |node| {
                        for _ in 0..num_expansions {
                            if node.is_fully_expanded() {
                                break;
                            }
                            black_box(node.clone().expand(black_box(&you)));
                        }
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

/// Benchmark expand overhead vs actual simulation
fn bench_expand_overhead(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = *compact.you_id();

    let mut group = c.benchmark_group("expand_overhead");

    // Benchmark just the expand call
    group.bench_function("full_expand", |b| {
        b.iter_batched(
            || Arc::new(Node::new_root(compact)),
            |node| {
                black_box(node.expand(black_box(&you)));
            },
            criterion::BatchSize::SmallInput,
        )
    });

    // Benchmark just node creation (to measure initialization overhead)
    group.bench_function("node_creation_only", |b| {
        b.iter(|| {
            black_box(Node::new_root(black_box(compact)));
        })
    });

    group.finish();
}

/// Benchmark expand in concurrent scenarios (multiple threads)
fn bench_expand_concurrent(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = *compact.you_id();

    let mut group = c.benchmark_group("expand_concurrent");
    group.sample_size(20); // Reduce sample size for concurrent benchmarks

    for num_threads in [2, 4, 8].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_threads", num_threads)),
            num_threads,
            |b, &num_threads| {
                b.iter_batched(
                    || Arc::new(Node::new_root(compact)),
                    |node| {
                        let handles: Vec<_> = (0..num_threads)
                            .map(|_| {
                                let node_clone = node.clone();
                                let you_clone = you;
                                std::thread::spawn(move || {
                                    for _ in 0..10 {
                                        if node_clone.is_fully_expanded() {
                                            break;
                                        }
                                        node_clone.clone().expand(&you_clone);
                                    }
                                })
                            })
                            .collect();

                        for handle in handles {
                            handle.join().unwrap();
                        }
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

/// Benchmark expand with different board complexities
fn bench_expand_complexity(c: &mut Criterion) {
    // This would ideally test boards with varying numbers of possible moves
    // For now, we use different game states as proxies for complexity

    let scenarios = vec![
        (
            "simple_2_snakes",
            include_str!("../../battlesnake-game-types/fixtures/start_of_game.json"),
        ),
        (
            "complex_4_snakes",
            include_str!("../../battlesnake-game-types/fixtures/4_snake_game.json"),
        ),
        (
            "constrained_cornered",
            include_str!("../../battlesnake-game-types/fixtures/cornered.json"),
        ),
    ];

    let mut group = c.benchmark_group("expand_complexity");

    for (name, fixture) in scenarios {
        let g: Result<DEGame, _> = serde_json::from_slice(fixture.as_bytes());
        if let Ok(g) = g {
            let snake_id_mapping = build_snake_id_map(&g);
            if let Ok(compact) = g.as_cell_board(&snake_id_mapping) {
                let compact: CellBoard4Snakes11x11 = compact;
                let you = *compact.you_id();

                group.bench_with_input(
                    BenchmarkId::from_parameter(name),
                    &compact,
                    |b, compact| {
                        b.iter_batched(
                            || Arc::new(Node::new_root(*compact)),
                            |node| {
                                // Expand 10 times to get a better average
                                for _ in 0..10 {
                                    if node.is_fully_expanded() {
                                        break;
                                    }
                                    black_box(node.clone().expand(black_box(&you)));
                                }
                            },
                            criterion::BatchSize::SmallInput,
                        );
                    },
                );
            }
        }
    }
    group.finish();
}

/// Benchmark memory allocation patterns during expand
fn bench_expand_allocations(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = *compact.you_id();

    c.bench_function("expand_allocations", |b| {
        b.iter_batched(
            || Arc::new(Node::new_root(compact)),
            |node| {
                // Expand multiple times to measure allocation patterns
                for _ in 0..20 {
                    if node.is_fully_expanded() {
                        break;
                    }
                    black_box(node.clone().expand(black_box(&you)));
                }
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

/// Benchmark the NonPushableQueue::pop_front overhead within expand
fn bench_expand_pop_front_overhead(c: &mut Criterion) {
    let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
    let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
    let g = g.expect("the json literal is valid");
    let snake_id_mapping = build_snake_id_map(&g);
    let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
    let you = *compact.you_id();

    let mut group = c.benchmark_group("expand_pop_front");

    // Benchmark first expansion (queue is full)
    group.bench_function("first_expand", |b| {
        b.iter_batched(
            || Arc::new(Node::new_root(compact)),
            |node| {
                black_box(node.expand(black_box(&you)));
            },
            criterion::BatchSize::SmallInput,
        )
    });

    // Benchmark middle expansion (queue partially consumed)
    group.bench_function("middle_expand", |b| {
        b.iter_batched(
            || {
                let node = Arc::new(Node::new_root(compact));
                // Consume half the queue
                for _ in 0..50 {
                    if node.is_fully_expanded() {
                        break;
                    }
                    node.clone().expand(&you);
                }
                node
            },
            |node| {
                black_box(node.expand(black_box(&you)));
            },
            criterion::BatchSize::SmallInput,
        )
    });

    // Benchmark last expansion (queue nearly empty)
    group.bench_function("last_expand", |b| {
        b.iter_batched(
            || {
                let node = Arc::new(Node::new_root(compact));
                // Consume almost all of the queue
                while !node.is_fully_expanded() {
                    node.clone().expand(&you);
                }
                // Create a fresh node for the actual benchmark
                Arc::new(Node::new_root(compact))
            },
            |node| {
                // Expand until nearly exhausted, then benchmark the last one
                while !node.is_fully_expanded() {
                    node.clone().expand(&you);
                }
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_expand_single,
    bench_expand_until_full,
    bench_expand_scenarios,
    bench_expand_n_times,
    bench_expand_overhead,
    bench_expand_concurrent,
    bench_expand_complexity,
    bench_expand_allocations,
    bench_expand_pop_front_overhead,
);
criterion_main!(benches);
