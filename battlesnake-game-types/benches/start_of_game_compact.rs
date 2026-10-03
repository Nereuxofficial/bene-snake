use arrayvec::ArrayVec;
use battlesnake_game_types::{
    compact_representation::StandardCellBoard4Snakes11x11,
    types::{
        Move, ReasonableMovesGame, SimulableGame, SnakeIDGettableGame, SnakeId, build_snake_id_map,
    },
    wire_representation::Game,
};
use criterion::{Criterion, criterion_group, criterion_main};
use std::{hint::black_box, time::Duration};
fn start_board() -> StandardCellBoard4Snakes11x11 {
    let game: Game =
        serde_json::from_str(include_str!("../fixtures/start_of_game.json")).expect("fixture");
    let ids = build_snake_id_map(&game);
    game.as_cell_board(&ids).expect("compact board")
}
fn bench_single_action(c: &mut Criterion) {
    let board = start_board();
    let moves: ArrayVec<(SnakeId, Move), 4> = board
        .reasonable_moves_for_each_snake()
        .into_iter()
        .filter_map(|(id, moves)| moves.first().map(|mv| (id, *mv)))
        .collect();
    c.bench_function("simulate_single_action/start_of_game", |b| {
        b.iter(|| black_box(black_box(&board).simulate_single_action(black_box(&moves))))
    });
}
fn bench_all_actions(c: &mut Criterion) {
    let board = start_board();
    let ids = board.get_snake_ids();
    // General enumeration is a separate utility workload, not the MCTS hot path.
    c.bench_function("enumerate_all_joint_actions/start_of_game", |b| {
        b.iter(|| {
            black_box(&board)
                .simulate(black_box(&ids))
                .for_each(|result| {
                    black_box(result);
                });
        });
    });
}
criterion_group! {
    name = benches;
    config = Criterion::default().warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(5));
    targets = bench_single_action, bench_all_actions
}
criterion_main!(benches);
