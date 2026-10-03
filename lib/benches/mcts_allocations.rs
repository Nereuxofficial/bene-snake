mod support;
use alloc_tracker::{Allocator, Session};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use lib::mcts::{Node, SearchDepthStats, bench::rollout_with_rng};
use std::{
    alloc::System,
    hint::black_box,
    time::{Duration, Instant},
};
#[global_allocator]
static ALLOCATOR: Allocator<System> = Allocator::system();
fn bench_allocations(c: &mut Criterion) {
    let fixtures = support::fixtures();
    let mut rollout = c.benchmark_group("allocations/rollout_seeded");
    for f in &fixtures {
        let node = Node::new_root(f.board);
        let mut stats = SearchDepthStats::default();
        let session = Session::new();
        let op = session.operation(format!("rollout_seeded_{}", f.name));
        rollout.bench_function(&f.name, |b| {
            b.iter_custom(|iters| {
                let mut rng = support::rng();
                let start = Instant::now();
                let span = op.measure_thread().iterations(iters);
                for _ in 0..iters {
                    black_box(rollout_with_rng(&node, &f.you, &mut stats, &mut rng));
                }
                drop(span);
                start.elapsed()
            })
        });
    }
    rollout.finish();
    let mut cold = c.benchmark_group("allocations/search_cold_first_iteration");
    for f in &fixtures {
        let session = Session::new();
        let op = session.operation(format!("search_cold_first_iteration_{}", f.name));
        cold.bench_function(&f.name, |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                let span = op.measure_thread().iterations(iters);
                for _ in 0..iters {
                    let mut s = support::Search::new(f, 0);
                    s.run(f.you, 1);
                    black_box(s.stats.iterations);
                }
                drop(span);
                start.elapsed()
            })
        });
    }
    cold.finish();
    let mut warm = c.benchmark_group("allocations/search_warm_iterations");
    warm.throughput(Throughput::Elements(support::SEARCH_BATCH));
    for f in &fixtures {
        let session = Session::new();
        let op = session.operation(format!("search_warm_iterations_{}", f.name));
        warm.bench_function(&f.name, |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut s = support::Search::new(f, support::WARMUP_ITERATIONS);
                    let start = Instant::now();
                    let span = op.measure_thread().iterations(support::SEARCH_BATCH);
                    s.run(f.you, support::SEARCH_BATCH);
                    drop(span);
                    elapsed += start.elapsed();
                    black_box(s.stats.iterations);
                }
                elapsed
            })
        });
    }
    warm.finish();
    let mut simulation = c.benchmark_group("allocations/simulate_single_action_batch");
    for f in &fixtures {
        let n = f.actions.len() as u64;
        simulation.throughput(Throughput::Elements(n));
        let session = Session::new();
        let op = session.operation(format!("simulate_single_action_{}", f.name));
        simulation.bench_function(&f.name, |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                let span = op.measure_thread().iterations(iters * n);
                for _ in 0..iters {
                    for action in &f.actions {
                        black_box(black_box(&f.board).simulate_single_action(black_box(action)));
                    }
                }
                drop(span);
                start.elapsed()
            })
        });
    }
    simulation.finish();
}
criterion_group! {name=benches;config=Criterion::default().warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(5));targets=bench_allocations}
criterion_main!(benches);
