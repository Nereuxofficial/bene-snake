mod support;
use battlesnake_game_types::{
    compact_representation::standard::moves_from_mask, types::HeadGettableGame,
};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use lib::{eval::evaluate_board_with_food, mcts::bench::sample_rollout_moves};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};
fn bench_hotpaths(c: &mut Criterion) {
    let fixtures = support::fixtures();
    let mut masks = c.benchmark_group("legal_move_masks");
    for f in &fixtures {
        masks.bench_function(&f.name, |b| {
            b.iter(|| black_box(black_box(&f.board).reasonable_move_masks()))
        });
    }
    masks.finish();
    let mut mobility = c.benchmark_group("candidate_neighbor_count");
    for f in &fixtures {
        let mut destinations = Vec::new();
        for (id, mask) in f.board.reasonable_move_masks() {
            let head = f.board.get_head_as_native_position(&id);
            for mv in moves_from_mask(mask) {
                destinations.push(f.board.describe_move(id, head, mv).destination);
            }
        }
        mobility.throughput(Throughput::Elements(destinations.len() as u64));
        mobility.bench_function(&f.name, |b| {
            b.iter(|| {
                for pos in black_box(&destinations) {
                    black_box(black_box(&f.board).free_neighbor_count(*pos));
                }
            })
        });
    }
    mobility.finish();
    let mut policy = c.benchmark_group("rollout_policy_seeded");
    for f in &fixtures {
        policy.bench_function(&f.name, |b| {
            b.iter_custom(|iters| {
                let mut rng = support::rng();
                let start = Instant::now();
                for _ in 0..iters {
                    black_box(sample_rollout_moves(
                        black_box(&f.board),
                        black_box(&f.food),
                        &mut rng,
                    ));
                }
                start.elapsed()
            })
        });
    }
    policy.finish();
    let mut simulation = c.benchmark_group("simulate_single_action_batch");
    for f in &fixtures {
        simulation.throughput(Throughput::Elements(f.actions.len() as u64));
        simulation.bench_function(&f.name, |b| {
            b.iter(|| {
                for moves in black_box(&f.actions) {
                    black_box(black_box(&f.board).simulate_single_action(black_box(moves)));
                }
            })
        });
    }
    simulation.finish();
    let mut eval = c.benchmark_group("leaf_evaluate_supplied_food");
    for f in &fixtures {
        eval.bench_function(&f.name, |b| {
            b.iter(|| {
                black_box(evaluate_board_with_food(
                    black_box(&f.board),
                    black_box(&f.you),
                    black_box(&f.food),
                ))
            })
        });
    }
    eval.finish();
}
criterion_group! {name=benches;config=Criterion::default().warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(5));targets=bench_hotpaths}
criterion_main!(benches);
