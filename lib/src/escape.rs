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

const UNREACHABLE: u8 = u8::MAX;

// Each square is queued once; all scratch storage fits on the stack.
fn distances(available: &[bool; 121], start: u8) -> [u8; 121] {
    let mut distance = [UNREACHABLE; 121];
    let mut queue = [0u8; 121];
    queue[0] = start;
    distance[start as usize] = 0;
    let (mut read, mut count) = (0, 1);
    while read < count {
        let cell = queue[read];
        read += 1;
        for mv in Move::all() {
            if let Some(p) = neighbor(cell, mv)
                && available[p as usize]
                && distance[p as usize] == UNREACHABLE
            {
                distance[p as usize] = distance[cell as usize] + 1;
                queue[count] = p;
                count += 1;
            }
        }
    }
    distance
}

#[derive(Clone, Copy)]
struct Arrival {
    distance: u8,
    length: u16,
}

struct Space {
    capacity: usize,
    exits: u8,
    food_distance: Option<u8>,
}

fn region(
    mut available: [bool; 121],
    food: &[bool; 121],
    rivals: &[Arrival; 121],
    body: Body,
    health: u8,
) -> Space {
    // Block the actual moved body, including stacked growth, while letting the old tail vacate.
    for p in body.cells.iter().take(body.len).skip(1) {
        available[*p as usize] = false;
    }
    let start = body.cells[0];
    available[start as usize] = true;
    let distance = distances(&available, start);
    let capacity = distance.iter().filter(|&&d| d != UNREACHABLE).count();
    let exits = Move::all()
        .iter()
        .filter(|&&mv| neighbor(start, mv).is_some_and(|p| available[p as usize]))
        .count() as u8;
    let food_distance = distance
        .iter()
        .enumerate()
        .filter_map(|(cell, &d)| {
            // d is measured after our first move. Food at this move's destination has d=0.
            if !food[cell] || d == UNREACHABLE || d >= health {
                return None;
            }
            let arrival = d + 1;
            let rival = rivals[cell];
            let claimed = rival.distance < arrival
                || (rival.distance == arrival && usize::from(rival.length) >= body.len);
            (!claimed).then_some(d)
        })
        .min();
    Space {
        capacity,
        exits,
        food_distance,
    }
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
    // Root-only static routes: tails may open later, so these are preferences, never death proofs.
    let available = std::array::from_fn(|cell| board.cell_is_free(CellIndex::from_usize(cell)));
    let food = std::array::from_fn(|cell| board.is_food(&CellIndex::from_usize(cell)));
    let mut rivals = [Arrival {
        distance: UNREACHABLE,
        length: 0,
    }; 121];
    for id in (0..4)
        .map(SnakeId)
        .filter(|id| *id != you && board.get_health(id) > 0)
    {
        let routes = distances(&available, board.get_head_as_native_position(&id).0);
        for (cell, distance) in routes.into_iter().enumerate() {
            if distance == UNREACHABLE || distance > board.get_health(&id) {
                continue;
            }
            let length = board.get_length(&id);
            if distance < rivals[cell].distance
                || (distance == rivals[cell].distance && length > rivals[cell].length)
            {
                rivals[cell] = Arrival { distance, length };
            }
        }
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
            let space = region(available, &food, &rivals, next, board.get_health(&you));
            // Bound all preferences: static routes do not account for future tail release,
            // hazards or opponent movement. Unreachable/contested food gets no route bonus.
            // Cells behind a single door are worth far less than the same count behind
            // several, so discount capacity by the exit count before it scales the weight.
            // Four is the ceiling for an interior cell, leaving open areas unchanged.
            let capacity = space.capacity * usize::from(space.exits.clamp(1, 4)) / 4;
            let factor = 1 + (3 * capacity / next.len).min(3) as u8;
            let food_bonus = space.food_distance.map_or(0, |d| 24 / (1 + d));
            preferred[index] = preferred[index]
                .saturating_mul(factor)
                .saturating_add(food_bonus)
                .saturating_add(space.exits.min(3) * 2);
        }
    }
    // Local exits are preferences, not survival proofs: two exits can both close
    // while a one-exit route follows a moving tail. Keep that route available to
    // MCTS and the tactical filter; prune only the solo self-traps proved above.
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
    fn food_routes_follow_a_detour_and_exclude_unreachable_food() {
        let mut available = [true; 121];
        for y in 0..10 {
            available[y * 11 + 1] = false;
        }
        let mut food = [false; 121];
        food[2] = true;
        let rivals = [Arrival {
            distance: UNREACHABLE,
            length: 0,
        }; 121];
        let body = Body {
            cells: [0; 128],
            len: 1,
        };
        let space = region(available, &food, &rivals, body, 100);
        assert_eq!(space.food_distance, Some(22)); // Manhattan distance is only two.
        assert_eq!(space.exits, 1);
        assert_eq!(
            region(available, &food, &rivals, body, 22).food_distance,
            None
        );
        available[111] = false; // Seal the remaining opening in the wall.
        assert_eq!(
            region(available, &food, &rivals, body, 100).food_distance,
            None
        );
    }

    #[test]
    fn food_arrival_accounts_for_first_move_and_relative_length() {
        let available = [true; 121];
        let mut food = [false; 121];
        food[22] = true;
        let mut rivals = [Arrival {
            distance: UNREACHABLE,
            length: 0,
        }; 121];
        let mut body = Body {
            cells: [0; 128],
            len: 3,
        };
        body.cells[..3].copy_from_slice(&[12, 1, 0]);
        assert_eq!(
            region(available, &food, &rivals, body, 100).food_distance,
            Some(2)
        );
        rivals[22] = Arrival {
            distance: 3,
            length: 2,
        };
        assert_eq!(
            region(available, &food, &rivals, body, 100).food_distance,
            Some(2)
        );
        rivals[22].length = 3;
        assert_eq!(
            region(available, &food, &rivals, body, 100).food_distance,
            None
        );
        rivals[22] = Arrival {
            distance: 2,
            length: 1,
        };
        assert_eq!(
            region(available, &food, &rivals, body, 100).food_distance,
            None
        );
        food[12] = true; // Immediate food is reachable at health one, before starvation.
        assert_eq!(
            region(available, &food, &rivals, body, 1).food_distance,
            Some(0)
        );
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
    fn corridor_preferences_keep_recorded_four_turn_escapes() {
        use crate::{
            mcts::Node,
            tactical::{Limits, MoveVerdict},
        };

        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/corridor-escapes.json")).unwrap();
        let parse_move = |value: &serde_json::Value| {
            Move::all()
                .into_iter()
                .find(|m| serde_json::to_value(m).unwrap() == *value)
                .unwrap()
        };
        for case in fixtures["cases"].as_array().unwrap() {
            let game: Game = serde_json::from_value(case["game"].clone()).unwrap();
            let escape = parse_move(&case["escape"]);
            let exposed = parse_move(&case["exposed"]);
            let ids = build_snake_id_map(&game);
            let you = ids[&game.you.id];
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
            let root = Node::new_root(board);
            let (mask, _) = root.root_candidates(you).unwrap();
            assert_ne!(
                mask & (1 << escape.as_index()),
                0,
                "{} turn {} discarded the escape {escape}",
                game.game.id,
                game.turn
            );

            // Check the finite-horizon label, without a machine-dependent time limit.
            // This does not assert that the stochastic search must choose one move.
            let analysis = crate::tactical::analyze(&board, you, mask, &Limits::for_test(4));
            assert!(matches!(
                analysis.verdict(escape),
                MoveVerdict::ProvenSafe { horizon: 4 }
            ));
            assert!(matches!(
                analysis.verdict(exposed),
                MoveVerdict::Exposed { .. }
            ));

            // The duel filter must see the restored sibling and can then reject
            // the exposed direction, rather than receiving a forced losing move.
            if game.board.snakes.len() == 2 {
                let filter = root.tactical_root_filter(you, &Limits::for_test(4));
                assert!(filter.applied);
                assert_eq!(
                    root.best_move_with_root_filter(you, Some(&filter)),
                    Some(escape)
                );
            }
        }
    }

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
