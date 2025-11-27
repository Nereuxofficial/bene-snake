use std::borrow::Cow;

use battlesnake_game_types::types::{ReasonableMovesGame, SimulatorInstruments, SnakeId};

use crate::{evaluate_board, CellBoard4Snakes11x11};

trait Score {
    fn evaluate(board: &CellBoard4Snakes11x11, you: &SnakeId, snake_ids: Cow<[SnakeId]>) -> u16 {
        evaluate_board(board, you, snake_ids)
    }
}

struct Node<S> {
    board: CellBoard4Snakes11x11,
    score: S,
}
#[derive(Debug)]
struct Instr;
impl SimulatorInstruments for Instr {
    fn observe_simulation(&self, _: std::time::Duration) {}
}
impl<T> Node<T> {
    pub fn explore_fixed(&self, snake_ids: Cow<[SnakeId]>) {
        let mut moves = self.board.reasonable_moves_for_each_snake();
        //self.board.simulate_with_moves(&Instr, &mut mv);
    }
}
