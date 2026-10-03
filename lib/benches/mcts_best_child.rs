mod support;
use criterion::{Criterion, criterion_group, criterion_main};
use std::{hint::black_box, time::Duration};
fn bench_best_move(c: &mut Criterion) {
    let mut group = c.benchmark_group("best_move_warm_tree");
    for f in support::fixtures() {
        let search = support::Search::new(&f, support::WARMUP_ITERATIONS);
        group.bench_function(&f.name, |b| {
            b.iter(|| black_box(search.root.best_move(black_box(f.you))))
        });
    }
    group.finish();
}
criterion_group! {name=benches;config=Criterion::default().warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(5));targets=bench_best_move}
criterion_main!(benches);
