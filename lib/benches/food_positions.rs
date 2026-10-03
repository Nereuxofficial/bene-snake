mod support;
use battlesnake_game_types::types::FoodGettableGame;
use criterion::{Criterion, criterion_group, criterion_main};
use lib::eval::{evaluate_board, evaluate_board_with_food};
use std::{hint::black_box, time::Duration};
fn bench_food_positions(c: &mut Criterion) {
    let fixtures = support::fixtures();
    let mut group = c.benchmark_group("food_positions_scan");
    for f in &fixtures {
        group.bench_function(&f.name, |b| {
            b.iter(|| black_box(black_box(&f.board).get_all_food_as_positions()))
        });
    }
    group.finish();
    let mut scan = c.benchmark_group("evaluate_with_food_scan");
    for f in &fixtures {
        scan.bench_function(&f.name, |b| {
            b.iter(|| black_box(evaluate_board(black_box(&f.board), black_box(&f.you))))
        });
    }
    scan.finish();
    let mut supplied = c.benchmark_group("evaluate_with_supplied_food");
    for f in &fixtures {
        supplied.bench_function(&f.name, |b| {
            b.iter(|| {
                black_box(evaluate_board_with_food(
                    black_box(&f.board),
                    black_box(&f.you),
                    black_box(&f.food),
                ))
            })
        });
    }
    supplied.finish();
}
criterion_group! {name=benches;config=Criterion::default().warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(5));targets=bench_food_positions}
criterion_main!(benches);
