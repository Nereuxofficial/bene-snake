use std::borrow::Borrow;

use arrayvec::ArrayVec;
use itertools::Itertools;
use tracing::instrument;

use crate::types::{Action, Move, N_MOVES, SnakeId};

use super::{CellBoard, CellNum, cell_board::EvaluateMode, dimensions::Dimensions};

/// Simulate one chosen move per snake without building a Cartesian product.
#[instrument(level = "trace", skip_all)]
pub fn simulate_single_action<
    T: CellNum,
    D: Dimensions,
    const BOARD_SIZE: usize,
    const MAX_SNAKES: usize,
>(
    board: &CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    moves: &[(SnakeId, Move)],
    evaluate_mode: EvaluateMode,
) -> (Action<MAX_SNAKES>, CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>) {
    let selected: ArrayVec<_, MAX_SNAKES> = moves.iter().map(|(id, mv)| (*id, [*mv])).collect();
    let states = board.generate_state(selected.iter(), evaluate_mode);
    let action = Action::collect_from(moves.iter());
    let game = board.evaluate_moves_with_state(moves.iter(), &states);
    if !game.assert_consistency() {
        panic!(
            "caught an inconsistent simulate, moves: {:?} orig: {}, new: {}",
            moves, board, game
        );
    }
    (action, game)
}

#[instrument(level = "trace", skip_all)]
pub fn simulate_with_moves<
    'a,
    S,
    T: CellNum,
    D: Dimensions,
    const BOARD_SIZE: usize,
    const MAX_SNAKES: usize,
>(
    board: &'a CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    snake_ids_and_moves: &[(SnakeId, S)],
    evaluate_mode: EvaluateMode,
) -> Box<dyn Iterator<Item = (Action<MAX_SNAKES>, CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>)> + 'a>
where
    S: Borrow<[Move]>,
{
    let mut snake_ids_we_are_simulating = [false; MAX_SNAKES];
    for (snake_id, _) in snake_ids_and_moves.iter() {
        snake_ids_we_are_simulating[snake_id.0.as_usize()] = true;
    }

    // [
    // sid major, move minor
    // [ some_reulst_struct, some_dead_struct ]
    // [ some_dead_struct, some_dead_struct ] // snake we didn't simulate
    let states = board.generate_state(snake_ids_and_moves.iter(), evaluate_mode);
    let mut dead_snakes_table = [[false; N_MOVES]; MAX_SNAKES];

    for (sid, result_row) in states.iter().enumerate() {
        for (move_index, move_result) in result_row.iter().enumerate() {
            dead_snakes_table[sid][move_index] = move_result.is_dead();
        }
    }

    let ids_and_moves_product = snake_ids_and_moves
        .iter()
        .map(|(snake_id, moves)| {
            let first_move = moves.borrow()[0];
            let mvs = moves
                .borrow()
                .iter()
                .filter(|mv| !dead_snakes_table[snake_id.0 as usize][mv.as_index()])
                .map(|mv| (*snake_id, *mv))
                .collect_vec();
            if mvs.is_empty() {
                vec![(*snake_id, first_move)]
            } else {
                mvs
            }
        })
        .multi_cartesian_product();
    let results = ids_and_moves_product.into_iter().map(move |m| {
        let action = Action::collect_from(m.iter());

        let game = board.evaluate_moves_with_state(m.iter(), &states);
        if !game.assert_consistency() {
            panic!(
                "caught an inconsistent simulate, moves: {:?} orig: {}, new: {}",
                m, board, game
            );
        }
        (action, game)
    });
    Box::new(results)
}
