mod support;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use lib::mcts::{Node, SearchDepthStats, bench::rollout_with_rng};
use std::{hint::black_box, time::Duration};
fn bench_rollout(c: &mut Criterion) {
    let mut group = c.benchmark_group("rollout_seeded");
    group.throughput(Throughput::Elements(1));
    for f in support::fixtures() {
        let node = Node::new_root(f.board);
        let mut stats = SearchDepthStats::default();
        group.bench_function(&f.name, |b| {
            b.iter_custom(|iters| {
                let mut rng = support::rng();
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    black_box(rollout_with_rng(
                        &node,
                        black_box(&f.you),
                        &mut stats,
                        &mut rng,
                    ));
                }
                start.elapsed()
            })
        });
    }
    group.finish();
    let mut production = c.benchmark_group("rollout_production_rng");
    for f in support::fixtures() {
        let node = Node::new_root(f.board);
        let mut stats = SearchDepthStats::default();
        let mut rng = rand::rng();
        production.bench_function(&f.name, |b| {
            b.iter(|| {
                black_box(rollout_with_rng(
                    &node,
                    black_box(&f.you),
                    &mut stats,
                    &mut rng,
                ))
            })
        });
    }
    production.finish();
}
criterion_group! {name=benches;config=Criterion::default().warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(5));targets=bench_rollout}
criterion_main!(benches);
