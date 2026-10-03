mod support;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::{hint::black_box, time::Duration};
fn bench_search(c: &mut Criterion) {
    let fixtures = support::fixtures();
    let mut roots = c.benchmark_group("root_prepare");
    for f in &fixtures {
        roots.bench_function(&f.name, |b| {
            b.iter_batched_ref(
                || lib::mcts::Node::new_root(f.board),
                |node| lib::mcts::bench::prepare_root(black_box(node), f.you),
                BatchSize::SmallInput,
            )
        });
    }
    roots.finish();
    let mut group = c.benchmark_group("search_fixed_iterations");
    for f in &fixtures {
        for (phase, warmup, count) in [
            ("cold", 0, 1),
            ("growing", 0, support::SEARCH_BATCH),
            ("decision", 0, support::DECISION_ITERATIONS),
            ("warm", support::WARMUP_ITERATIONS, support::SEARCH_BATCH),
        ] {
            group.throughput(Throughput::Elements(count));
            group.bench_function(BenchmarkId::new(phase, &f.name), |b| {
                b.iter_batched_ref(
                    || support::Search::new(f, warmup),
                    |s| {
                        s.run(black_box(f.you), count);
                        black_box(s.stats.iterations);
                    },
                    BatchSize::LargeInput,
                )
            });
        }
    }
    group.finish();
}
criterion_group! {name=benches;config=Criterion::default().warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(5));targets=bench_search}
criterion_main!(benches);
