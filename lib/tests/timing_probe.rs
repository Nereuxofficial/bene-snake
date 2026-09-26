use std::{hint::black_box, sync::Arc, time::Instant};

use battlesnake_game_types::{
    compact_representation::{
        standard::CellBoard4Snakes11x11, wrapped::CellBoard4SnakesSquare11x11,
    },
    types::{
        HeadGettableGame, NeighborDeterminableGame, RandomReasonableMovesGame, ReasonableMovesGame,
        SnakeId, YouDeterminableGame, build_snake_id_map,
    },
    wire_representation::Game,
};
use lib::{
    eval::evaluate_board,
    mcts::{Node, SearchDepthStats, search_once},
};

fn start_board() -> CellBoard4Snakes11x11 {
    let game: Game = serde_json::from_str(include_str!(
        "../../battlesnake-game-types/fixtures/start_of_game.json"
    ))
    .expect("fixture");
    let ids = build_snake_id_map(&game);
    game.as_cell_board(&ids).expect("board")
}

fn timeit<F: FnMut()>(name: &str, iters: u32, mut f: F) {
    for _ in 0..iters / 10 {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let ns = start.elapsed().as_nanos() as f64 / iters as f64;
    println!("{name:>34}: {ns:8.1} ns/iter");
}

#[test]
fn timing_probe() {
    let board = start_board();
    let you = *board.you_id();

    let wrapped_game: Game = serde_json::from_str(include_str!(
        "../../battlesnake-game-types/fixtures/wrapped_fixture.json"
    ))
    .expect("fixture");
    let wrapped_ids = build_snake_id_map(&wrapped_game);
    let wrapped = CellBoard4SnakesSquare11x11::convert_from_game(wrapped_game, &wrapped_ids)
        .expect("wrapped board");

    timeit("Instant::now()", 1_000_000, || {
        black_box(Instant::now());
    });

    timeit("assert_consistency()", 1_000_000, || {
        black_box(wrapped.assert_consistency());
    });

    let moves = [
        (SnakeId(0), battlesnake_game_types::types::Move::Up),
        (SnakeId(1), battlesnake_game_types::types::Move::Right),
        (SnakeId(2), battlesnake_game_types::types::Move::Down),
        (SnakeId(3), battlesnake_game_types::types::Move::Left),
    ];
    timeit("simulate_single_action", 1_000_000, || {
        black_box(board.simulate_single_action(&moves));
    });

    let head = board.get_head_as_native_position(&you);
    timeit("neighbors().count()", 1_000_000, || {
        black_box(board.neighbors(&head).count());
    });

    timeit("evaluate_board", 1_000_000, || {
        black_box(evaluate_board(black_box(&board), black_box(&you)));
    });

    timeit("reasonable_moves_for_each_snake", 500_000, || {
        black_box(board.reasonable_moves_for_each_snake());
    });

    timeit("random_moves_per_snake", 500_000, || {
        let mut rng = rand::rng();
        black_box(
            board
                .random_reasonable_move_for_each_snake(&mut rng)
                .count(),
        );
    });

    let root = Arc::new(Node::new_root(board));
    let mut rollout_stats = SearchDepthStats::default();
    timeit("rollout", 200_000, || {
        black_box(Arc::clone(&root).rollout(black_box(&you), &mut rollout_stats));
    });

    let mut search_stats = SearchDepthStats::default();
    timeit("search_once (tree grows)", 50_000, || {
        search_once(&root, black_box(&you), &mut search_stats);
    });
}
