#![feature(iter_collect_into)]

use std::{alloc::System, hint::black_box, sync::Arc};

use alloc_tracker::{Allocator, Session};
use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        HeadGettableGame, Move, NeighborDeterminableGame, RandomReasonableMovesGame, SimulableGame,
        SnakeId, YouDeterminableGame, build_snake_id_map,
    },
    wire_representation::Game,
};
use lib::{
    eval::evaluate_board,
    mcts::{Node, SearchDepthStats, search_once},
};

#[global_allocator]
static ALLOCATOR: Allocator<System> = Allocator::system();

fn start_board() -> CellBoard4Snakes11x11 {
    let game: Game = serde_json::from_str(include_str!(
        "../../battlesnake-game-types/fixtures/start_of_game.json"
    ))
    .expect("fixture");
    let ids = build_snake_id_map(&game);
    game.as_cell_board(&ids).expect("board")
}

fn hungry_board() -> CellBoard4Snakes11x11 {
    let mut v: serde_json::Value = serde_json::from_str(include_str!(
        "../../battlesnake-game-types/fixtures/late_stage.json"
    ))
    .expect("fixture");
    v["you"]["health"] = serde_json::json!(25);
    for s in v["board"]["snakes"].as_array_mut().expect("snakes") {
        s["health"] = serde_json::json!(25);
    }
    let game: Game = serde_json::from_value(v).expect("game");
    let ids = build_snake_id_map(&game);
    game.as_cell_board(&ids).expect("board")
}

fn measure<F: FnMut()>(op_name: &str, iters: u64, mut f: F) {
    let session = Session::new();
    let op = session.operation(op_name);
    {
        let _span = op.measure_thread().iterations(iters);
        for _ in 0..iters {
            f();
        }
    }
    drop(session);
}

#[test]
fn probe() {
    let board = start_board();
    let you = *board.you_id();
    let root = Arc::new(Node::new_root(board));
    let mut rollout_stats = SearchDepthStats::default();
    let mut search_stats = SearchDepthStats::default();

    measure("node_new_root", 1000, || {
        black_box(Arc::new(Node::new_root(black_box(board))));
    });

    measure("rollout", 1000, || {
        black_box(Arc::clone(&root).rollout(black_box(&you), &mut rollout_stats));
    });

    measure("search_once_same_root", 1000, || {
        search_once(&root, black_box(&you), &mut search_stats);
    });

    measure("search_once_fresh_root", 500, || {
        let r = Arc::new(Node::new_root(board));
        search_once(&r, black_box(&you), &mut search_stats);
    });

    let hungry = hungry_board();
    measure("evaluate_board_satiated", 1000, || {
        black_box(evaluate_board(black_box(&board), black_box(&you)));
    });
    measure("evaluate_board_hungry", 1000, || {
        black_box(evaluate_board(black_box(&hungry), black_box(&you)));
    });

    let moves = [
        (SnakeId(0), Move::Up),
        (SnakeId(1), Move::Right),
        (SnakeId(2), Move::Down),
        (SnakeId(3), Move::Left),
    ];
    let moves_multi = [
        (SnakeId(0), [Move::Up]),
        (SnakeId(1), [Move::Right]),
        (SnakeId(2), [Move::Down]),
        (SnakeId(3), [Move::Left]),
    ];
    measure("simulate_single_action", 1000, || {
        black_box(board.simulate_single_action(&moves));
    });

    measure("random_moves_into_arrayvec", 1000, || {
        let mut rng = rand::rng();
        let mut out = arrayvec::ArrayVec::<(SnakeId, Move), 4>::new();
        board
            .random_reasonable_move_for_each_snake(&mut rng)
            .collect_into(&mut out);
        black_box(out);
    });

    measure("possible_moves_boxed", 1000, || {
        let head = board.get_head_as_native_position(&you);
        black_box(board.possible_moves(&head).count());
    });

    measure("simulate_with_moves_boxed", 1000, || {
        black_box(board.simulate_with_moves(&moves_multi).next());
    });

    // Report: dropping the probe session prints nothing, so print via a final
    // session that re-runs nothing. Instead re-run the interesting ones with stdout.
    println!("--- probe done ---");
}
