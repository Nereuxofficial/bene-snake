//! Bounded root checks. Area is a preference; only proved solo self-traps are pruned.
use battlesnake_game_types::{
    compact_representation::{CellIndex, standard::CellBoard4Snakes11x11},
    types::{
        FoodQueryableGame, HeadGettableGame, HealthGettableGame, LengthGettableGame, Move,
        SnakeBodyGettableGame, SnakeId,
    },
};

const HORIZON: usize = 8;
const MAX_STATES_PER_MOVE: usize = 2048;

pub struct EscapeAnalysis {
    pub mask: u8,
    pub weights: [u8; 4],
}

#[derive(Clone, Copy)]
struct Body {
    cells: [u8; 128],
    len: usize,
}

impl Body {
    fn step(self, destination: u8, grow: bool) -> Option<Self> {
        if self.len == 0
            || (self.len > 1 && destination == self.cells[1])
            || self.cells[..self.len.saturating_sub(1)].contains(&destination)
            || self.len + usize::from(grow) > self.cells.len()
        {
            return None;
        }
        let mut next = self;
        next.cells[1..self.len].copy_from_slice(&self.cells[..self.len - 1]);
        next.cells[0] = destination;
        if grow {
            // Standard simulation moves the tail, then doubles the new tail on food.
            next.cells[self.len] = next.cells[self.len - 1];
            next.len += 1;
        }
        Some(next)
    }
}

fn neighbor(cell: u8, mv: Move) -> Option<u8> {
    let x = cell % 11;
    match mv {
        Move::Up if cell < 110 => Some(cell + 11),
        Move::Down if cell >= 11 => Some(cell - 11),
        Move::Left if x > 0 => Some(cell - 1),
        Move::Right if x < 10 => Some(cell + 1),
        _ => None,
    }
}

fn can_escape(body: Body, remaining: usize, budget: &mut usize) -> bool {
    // Exhausting work means unknown, not dead. Future opponents, hazards, health
    // loss and food growth are ignored: this grants the snake extra freedom.
    if remaining == 0 || *budget == 0 {
        return true;
    }
    *budget -= 1;
    Move::all().iter().any(|mv| {
        neighbor(body.cells[0], *mv)
            .and_then(|p| body.step(p, false))
            .is_some_and(|next| can_escape(next, remaining - 1, budget))
    })
}

fn region(board: &CellBoard4Snakes11x11, body: Body, start: u8, cap: usize) -> usize {
    let mut available = [false; 121];
    for (cell, free) in available.iter_mut().enumerate() {
        // Single tails are optimistic exits for this soft preference. Stacked
        // tails and the moved snake's actual body remain blocked.
        *free = board.cell_is_free(CellIndex::from_usize(cell));
    }
    // The old head becomes body and the actual old tail may have moved away.
    for p in body.cells.iter().take(body.len).skip(1) {
        available[*p as usize] = false;
    }
    available[start as usize] = true;
    let mut seen = [false; 121];
    let mut queue = [0u8; 121];
    queue[0] = start;
    seen[start as usize] = true;
    let (mut read, mut count) = (0, 1);
    while read < count && count < cap {
        let cell = queue[read];
        read += 1;
        for mv in Move::all() {
            if let Some(p) = neighbor(cell, mv)
                && available[p as usize]
                && !seen[p as usize]
            {
                seen[p as usize] = true;
                queue[count] = p;
                count += 1;
                if count == cap {
                    break;
                }
            }
        }
    }
    count
}

pub fn analyze(
    board: &CellBoard4Snakes11x11,
    you: SnakeId,
    mask: u8,
    weights: [u8; 4],
) -> EscapeAnalysis {
    if board.get_health(&you) == 0 {
        return EscapeAnalysis { mask, weights };
    }
    let cells = board.get_snake_body_vec(&you);
    if cells.is_empty() || cells.len() >= 128 {
        return EscapeAnalysis { mask, weights };
    }
    let mut body = Body {
        cells: [0; 128],
        len: cells.len(),
    };
    for (slot, cell) in body.cells.iter_mut().zip(&cells) {
        *slot = cell.as_usize() as u8;
    }
    let mut surviving = 0;
    let mut preferred = weights;
    for mv in Move::all() {
        let index = mv.as_index();
        if mask & (1 << index) == 0 {
            continue;
        }
        let next = neighbor(body.cells[0], mv)
            .and_then(|p| body.step(p, board.is_food(&CellIndex::from_usize(p as usize))));
        if let Some(next) = next {
            let mut budget = MAX_STATES_PER_MOVE;
            // A short self-trap can still be a winning attack if it immediately
            // eliminates the last opponent. Preserve that possibility for MCTS.
            let mut opponents = (0..4)
                .map(SnakeId)
                .filter(|id| *id != you && board.get_health(id) > 0);
            let opponent = opponents.next().filter(|_| opponents.next().is_none());
            let could_win_now = opponent.is_some_and(|id| {
                cells.len() > usize::from(board.get_length(&id))
                    && Move::all().iter().any(|reply| {
                        neighbor(board.get_head_as_native_position(&id).0, *reply)
                            == Some(next.cells[0])
                    })
            });
            if could_win_now || can_escape(next, HORIZON - 1, &mut budget) {
                surviving |= 1 << index;
            }
            let capacity = region(board, next, next.cells[0], next.len.min(121));
            // Equal capacities leave priors unchanged relative to one another.
            // Bound the preference: a small static region may open as tails move.
            let factor = 1 + (3 * capacity / next.len).min(3) as u8;
            preferred[index] = preferred[index].saturating_mul(factor);
        }
    }
    EscapeAnalysis {
        mask: if surviving == 0 { mask } else { surviving },
        weights: preferred,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tail_following_and_stacked_growth_match_standard_motion() {
        let body = Body {
            cells: {
                let mut c = [0; 128];
                c[..4].copy_from_slice(&[12, 1, 0, 11]);
                c
            },
            len: 4,
        };
        let next = body.step(11, false).unwrap();
        assert_eq!(&next.cells[..4], &[11, 12, 1, 0]);
        let grown = body.step(23, true).unwrap();
        assert_eq!(&grown.cells[..5], &[23, 12, 1, 0, 0]);
        assert!(body.step(1, false).is_none());
        assert!(body.step(0, false).is_none());
    }
    #[test]
    fn exhausted_budget_does_not_claim_a_trap() {
        let body = Body {
            cells: [0; 128],
            len: 3,
        };
        assert!(can_escape(body, 8, &mut 0));
    }
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    use battlesnake_game_types::{
        types::{HeadGettableGame, ReasonableMovesGame, build_snake_id_map},
        wire_representation::Game,
    };
    #[test]
    fn rejects_recorded_self_traps_and_keeps_the_escape() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../fixtures/self-trap-approaches.json")).unwrap();
        for case in cases {
            let game: Game = serde_json::from_value(case["game"].clone()).unwrap();
            let ids = build_snake_id_map(&game);
            let you = ids[&game.you.id];
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
            let mask = board.reasonable_move_mask(board.get_head_as_native_position(&you));
            let result = analyze(&board, you, mask, [4; 4]);
            let parse_move = |value: &serde_json::Value| {
                Move::all()
                    .into_iter()
                    .find(|m| serde_json::to_value(m).unwrap() == *value)
                    .unwrap()
            };
            let bad = parse_move(&case["unsafe"]);
            let good = parse_move(&case["escape"]);
            assert_ne!(mask & (1 << bad.as_index()), 0);
            assert_eq!(
                result.mask & (1 << bad.as_index()),
                0,
                "{} turn {}",
                game.game.id,
                game.turn
            );
            assert_ne!(result.mask & (1 << good.as_index()), 0);
            let root = crate::mcts::Node::new_root(board);
            assert_ne!(root.best_move(you), Some(bad));
        }
    }
    #[test]
    fn first_step_growth_matches_compact_simulation() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../fixtures/self-trap-approaches.json")).unwrap();
        let mut game: Game = serde_json::from_value(cases[0]["game"].clone()).unwrap();
        game.board.snakes.retain(|s| s.id == game.you.id);
        game.board.food.clear();
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let mv = board.reasonable_moves_for_each_snake()[0].1[0];
        let dest = game.you.head.add_vec(mv.to_vector());
        game.board.food.push(dest);
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let original = board.get_snake_body_vec(&you);
        let mut body = Body {
            cells: [0; 128],
            len: original.len(),
        };
        for (slot, cell) in body.cells.iter_mut().zip(original) {
            *slot = cell.as_usize() as u8;
        }
        let step = body.step(CellIndex::<u8>::new(dest, 11).0, true).unwrap();
        let next = board.simulate_single_action(&[(you, mv)]).1;
        let actual: Vec<_> = next.get_snake_body_vec(&you).iter().map(|p| p.0).collect();
        assert_eq!(&step.cells[..step.len], actual);
    }
    #[test]
    fn forced_positions_keep_a_fallback_candidate() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../fixtures/self-trap-approaches.json")).unwrap();
        let game: Game = serde_json::from_value(cases[0]["game"].clone()).unwrap();
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let mask = 1 << Move::Down.as_index();
        assert_eq!(analyze(&board, you, mask, [4; 4]).mask, mask);
    }

    #[test]
    fn preserves_a_possible_immediate_win_over_the_last_opponent() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../fixtures/self-trap-approaches.json")).unwrap();
        let mut game: Game = serde_json::from_value(cases[0]["game"].clone()).unwrap();
        game.board.snakes.retain(|s| s.id == game.you.id);
        let destination = game.you.head.add_vec(Move::Down.to_vector());
        let attack_from = Move::all()
            .iter()
            .map(|m| destination.add_vec(m.to_vector()))
            .find(|p| !game.off_board(*p) && !game.you.body.contains(p))
            .unwrap();
        let mut opponent = game.you.clone();
        opponent.id = "last-opponent".into();
        opponent.head = attack_from;
        opponent.body = [attack_from; 3].into();
        game.board.snakes.push(opponent);
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let mask = (1 << Move::Down.as_index()) | (1 << Move::Left.as_index());
        assert_ne!(
            analyze(&board, you, mask, [4; 4]).mask & (1 << Move::Down.as_index()),
            0
        );
    }
}
