use std::{hint::black_box, time::Duration};

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{FoodGettableGame, YouDeterminableGame, build_snake_id_map},
    wire_representation::Game,
};
use criterion::{Criterion, criterion_group, criterion_main};
use lib::eval::evaluate_board;

fn board(fixture: &str) -> CellBoard4Snakes11x11 {
    let game: Game = serde_json::from_str(fixture).expect("valid game fixture");
    let snake_ids = build_snake_id_map(&game);
    game.as_cell_board(&snake_ids).expect("compact board")
}

fn bench_food_positions(c: &mut Criterion) {
    let mut group = c.benchmark_group("food_positions");
    for (name, fixture) in [
        (
            "start_of_game",
            include_str!("../../battlesnake-game-types/fixtures/start_of_game.json"),
        ),
        (
            "late_stage",
            include_str!("../../battlesnake-game-types/fixtures/late_stage.json"),
        ),
    ] {
        let board = board(fixture);
        group.bench_function(name, |b| {
            b.iter(|| {
                let food = black_box(&board).get_all_food_as_positions();
                black_box(food.iter().map(|p| p.x + p.y).sum::<i32>())
            })
        });
    }
    group.finish();
}

/// The late stage fixture with every snake hungry, so that `evaluate_board`
/// takes its food lookup branch.
fn hungry_board() -> CellBoard4Snakes11x11 {
    let mut fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../battlesnake-game-types/fixtures/late_stage.json"
    ))
    .expect("valid fixture");
    fixture["you"]["health"] = serde_json::json!(25);
    for snake in fixture["board"]["snakes"].as_array_mut().expect("snakes") {
        snake["health"] = serde_json::json!(25);
    }
    let game: Game = serde_json::from_value(fixture).expect("valid game");
    let snake_ids = build_snake_id_map(&game);
    game.as_cell_board(&snake_ids).expect("compact board")
}

fn bench_evaluate_board(c: &mut Criterion) {
    let mut group = c.benchmark_group("evaluate_board");
    for (name, board) in [
        (
            "satiated",
            board(include_str!(
                "../../battlesnake-game-types/fixtures/late_stage.json"
            )),
        ),
        ("hungry", hungry_board()),
    ] {
        let you = *board.you_id();
        group.bench_function(name, |b| {
            b.iter(|| evaluate_board(black_box(&board), black_box(&you)))
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(5));
    targets = bench_food_positions, bench_evaluate_board
}
criterion_main!(benches);
