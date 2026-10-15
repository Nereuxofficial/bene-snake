//! Bounded food-race estimates for root advice and leaf evaluation. Static routes are estimates, never survival proofs.
use battlesnake_game_types::{
    compact_representation::{CellIndex, standard::CellBoard4Snakes11x11},
    types::{
        FoodGettableGame, FoodQueryableGame, HazardQueryableGame, HeadGettableGame,
        HealthGettableGame, LengthGettableGame, Move, SnakeBodyGettableGame, SnakeId,
    },
};
use std::time::{Duration, Instant};

const UNREACHED: u8 = u8::MAX;
pub const MAX_EXPANSIONS: usize = 2048;
pub const TIME_LIMIT: Duration = Duration::from_millis(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaceClass {
    Favourable,
    Contested,
    LikelyLost,
    Unknown,
    BeyondHealth,
}

#[derive(Clone, Debug)]
pub struct FoodRace {
    pub cell: u8,
    pub class: RaceClass,
    pub arrival: Option<u8>,
    pub rival: Option<SnakeId>,
    pub rival_arrival: Option<u8>,
    pub rival_length: Option<u16>,
    pub health_margin: u8,
    pub uncertain_route: bool,
    pub first_moves: u8,
    score: i32,
}

#[derive(Debug)]
pub struct Assessment {
    pub foods: Vec<FoodRace>,
    pub expansions: usize,
    pub budget_exhausted: bool,
    pub best_chance: bool,
}

/// Persist only a soft target preference across three consecutive request turns.
#[derive(Default)]
pub struct TargetHistory {
    last: Option<(i32, Option<u8>)>,
}
impl TargetHistory {
    pub fn hint(&self, turn: i32) -> Option<u8> {
        self.last
            .filter(|(previous, _)| {
                turn.checked_sub(*previous)
                    .is_some_and(|d| (1..=3).contains(&d))
            })
            .and_then(|(_, cell)| cell)
    }
    pub fn observe(&mut self, turn: i32, target: Option<u8>) {
        if self.last.is_none_or(|(previous, _)| turn >= previous) {
            self.last = Some((turn, target));
        }
    }
}

impl Assessment {
    pub fn target(&self, hint: Option<u8>) -> Option<&FoodRace> {
        if self.budget_exhausted {
            return None;
        }
        let best = self
            .foods
            .iter()
            .filter(|f| f.arrival.is_some() && f.first_moves != 0)
            .max_by_key(|f| (f.score, std::cmp::Reverse(f.cell)))?;
        // Drop stale, adverse, or materially worse commitments immediately.
        self.foods
            .iter()
            .find(|f| {
                Some(f.cell) == hint
                    && f.arrival.is_some()
                    && f.first_moves != 0
                    && !matches!(f.class, RaceClass::LikelyLost | RaceClass::BeyondHealth)
                    && f.score >= best.score - 6
                    && f.health_margin.saturating_add(4) >= best.health_margin
            })
            .or(Some(best))
    }

    pub fn priors(&self, neutral: [u8; 4], health: u8, hint: Option<u8>) -> Option<([u8; 4], u8)> {
        let target = self.target(hint)?;
        let bonus = 12
            + if health <= 20 {
                12
            } else if health <= 50 {
                6
            } else {
                0
            };
        let weights = std::array::from_fn(|i| {
            if target.first_moves & (1 << i) != 0 {
                neutral[i].saturating_add(bonus)
            } else {
                neutral[i]
            }
        });
        Some((weights, target.cell))
    }
}

#[derive(Clone, Copy)]
struct Route {
    distance: u8,
    health: u8,
    margin: u8,
    growth: u8,
    uncertain: bool,
    first: u8,
}
const MISSING: Route = Route {
    distance: UNREACHED,
    health: 0,
    margin: 0,
    growth: 0,
    uncertain: true,
    first: 0,
};

fn neighbor(cell: u8, mv: Move) -> Option<u8> {
    match mv {
        Move::Up if cell < 110 => Some(cell + 11),
        Move::Down if cell >= 11 => Some(cell - 11),
        Move::Left if !cell.is_multiple_of(11) => Some(cell - 1),
        Move::Right if cell % 11 < 10 => Some(cell + 1),
        _ => None,
    }
}

struct Work {
    expansions: usize,
    max: usize,
    deadline: Option<Instant>,
}
impl Work {
    fn spend(&mut self) -> bool {
        if self.expansions >= self.max || self.deadline.is_some_and(|d| Instant::now() >= d) {
            return false;
        }
        self.expansions += 1;
        true
    }
}

fn routes(
    board: &CellBoard4Snakes11x11,
    id: SnakeId,
    first_mask: u8,
    tails: &[bool; 121],
    work: &mut Work,
    track_first_moves: bool,
) -> Option<[Route; 121]> {
    let start = board.get_head_as_native_position(&id).0;
    let mut paths = [MISSING; 121];
    paths[start as usize] = Route {
        distance: 0,
        health: board.get_health(&id),
        margin: board.get_health(&id),
        growth: 0,
        uncertain: false,
        first: 0,
    };
    let mut queue = [0u8; MAX_EXPANSIONS * 4 + 1];
    queue[0] = start;
    let (mut read, mut write) = (0, 1);
    while read < write {
        if !work.spend() {
            return None;
        }
        let cell = queue[read];
        read += 1;
        let here = paths[cell as usize];
        for mv in Move::all() {
            if cell == start && first_mask & (1 << mv.as_index()) == 0 {
                continue;
            }
            let Some(next) = neighbor(cell, mv) else {
                continue;
            };
            let native = CellIndex::from_usize(next as usize);
            if !board.cell_is_free(native) {
                continue;
            }
            let food = board.is_food(&native);
            let cost = 1u16
                + if board.is_hazard(&native) {
                    u16::from(board.get_hazard_damage())
                } else {
                    0
                };
            let health = if food {
                100
            } else {
                u16::from(here.health).saturating_sub(cost) as u8
            };
            if health == 0 {
                continue;
            }
            let candidate = Route {
                distance: here.distance + 1,
                health,
                margin: u16::from(here.health).saturating_sub(cost) as u8,
                growth: here.growth + u8::from(food),
                uncertain: here.uncertain || tails[next as usize] || here.growth > 0,
                first: if cell == start {
                    1 << mv.as_index()
                } else {
                    here.first
                },
            };
            let old = paths[next as usize];
            // One shortest static route label per cell. Longer routes via food may
            // exist; not finding one is unknown, not a proof of unreachability.
            if candidate.distance > old.distance {
                continue;
            }
            let first = if track_first_moves && candidate.distance == old.distance {
                old.first | candidate.first
            } else {
                candidate.first
            };
            if candidate.distance < old.distance
                || candidate.health > old.health
                || (track_first_moves && candidate.distance == old.distance && first != old.first)
                || (candidate.distance == old.distance && old.uncertain && !candidate.uncertain)
            {
                let mut chosen =
                    if candidate.distance < old.distance || candidate.health >= old.health {
                        candidate
                    } else {
                        old
                    };
                chosen.first = first;
                paths[next as usize] = chosen;
                if write == queue.len() {
                    return None;
                }
                queue[write] = next;
                write += 1;
            }
        }
    }
    Some(paths)
}

pub fn assess(
    board: &CellBoard4Snakes11x11,
    you: SnakeId,
    mask: u8,
    deadline: Option<Instant>,
    max_expansions: usize,
) -> Assessment {
    assess_routes(board, you, mask, deadline, max_expansions, true)
}

/// Leaf evaluation needs arrival classes, not unions of root directions. Avoid
/// re-expanding equal-distance labels merely to propagate another first move.
/// This still tracks health, growth and tail uncertainty on the actual board.
pub(crate) fn assess_for_evaluation(
    board: &CellBoard4Snakes11x11,
    you: SnakeId,
    mask: u8,
) -> Assessment {
    assess_routes(board, you, mask, None, MAX_EXPANSIONS, false)
}

fn assess_routes(
    board: &CellBoard4Snakes11x11,
    you: SnakeId,
    mask: u8,
    deadline: Option<Instant>,
    max_expansions: usize,
    track_first_moves: bool,
) -> Assessment {
    let mut result = Assessment {
        foods: Vec::new(),
        expansions: 0,
        budget_exhausted: false,
        best_chance: false,
    };
    let foods = board.get_all_food_as_positions();
    if foods.is_empty() {
        result.best_chance = true;
        return result;
    }
    let mut work = Work {
        expansions: 0,
        max: max_expansions.min(MAX_EXPANSIONS),
        deadline,
    };
    let mut tails = [false; 121];
    for id in (0..4).map(SnakeId).filter(|id| board.get_health(id) > 0) {
        if let Some(tail) = board.get_snake_body_vec(&id).last() {
            tails[tail.as_usize()] = true;
        }
    }
    let mut all = [[MISSING; 121]; 4];
    for id in (0..4).map(SnakeId).filter(|id| board.get_health(id) > 0) {
        let first = if id == you {
            mask
        } else {
            board.reasonable_move_mask(board.get_head_as_native_position(&id))
        };
        let Some(paths) = routes(board, id, first, &tails, &mut work, track_first_moves) else {
            result.budget_exhausted = true;
            result.expansions = work.expansions;
            return result;
        };
        all[id.as_usize()] = paths;
    }
    for food in &foods {
        if !work.spend() {
            result.budget_exhausted = true;
            result.expansions = work.expansions;
            return result;
        }
        let cell = (food.y * 11 + food.x) as u8;
        let own = all[you.as_usize()][cell as usize];
        let rival = (0..4)
            .map(SnakeId)
            .filter(|id| *id != you && board.get_health(id) > 0)
            .filter_map(|id| {
                let r = all[id.as_usize()][cell as usize];
                (r.distance != UNREACHED).then_some((id, r))
            })
            .min_by_key(|(id, r)| {
                (
                    r.distance,
                    std::cmp::Reverse(board.get_length(id) + u16::from(r.growth)),
                )
            });
        let own_length = board.get_length(&you) + u16::from(own.growth);
        let rival_length = rival.map(|(id, r)| board.get_length(&id) + u16::from(r.growth));
        let class = if own.distance == UNREACHED {
            let head = board.get_head_as_position(&you);
            let lower = (head.x - food.x).abs() + (head.y - food.y).abs();
            if foods.len() == 1 && lower > i32::from(board.get_health(&you)) {
                RaceClass::BeyondHealth
            } else {
                RaceClass::Unknown
            }
        } else if own.uncertain || rival.is_some_and(|(_, r)| r.uncertain) {
            RaceClass::Contested
        } else if rival.is_some_and(|(_, r)| {
            r.distance < own.distance
                || (r.distance == own.distance && rival_length.unwrap() >= own_length)
        }) {
            RaceClass::LikelyLost
        } else {
            RaceClass::Favourable
        };
        let arrival = (own.distance != UNREACHED).then_some(own.distance);
        let health_margin = if arrival.is_some() {
            // Health *before* target food restores it.
            own.margin
        } else {
            0
        };
        let grade = match class {
            RaceClass::Favourable => 120,
            RaceClass::Contested => 70,
            RaceClass::Unknown => 30,
            RaceClass::LikelyLost => 20,
            RaceClass::BeyondHealth => -100,
        };
        let margin = rival
            .map_or(4, |(_, r)| i32::from(r.distance) - i32::from(own.distance))
            .clamp(-4, 4);
        let exits = board.free_neighbor_count(CellIndex::from_usize(cell as usize));
        let score = grade - 3 * i32::from(own.distance.min(121))
            + i32::from(health_margin.min(20))
            + margin * 4
            + i32::from(exits) * 2;
        result.foods.push(FoodRace {
            cell,
            class,
            arrival,
            rival: rival.map(|(id, _)| id),
            rival_arrival: rival.map(|(_, r)| r.distance),
            rival_length,
            health_margin,
            uncertain_route: own.uncertain,
            first_moves: own.first,
            score,
        });
    }
    result.expansions = work.expansions;
    result.best_chance = !result.foods.iter().any(|f| {
        matches!(f.class, RaceClass::Favourable | RaceClass::Contested) && f.arrival.is_some()
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use battlesnake_game_types::{
        types::build_snake_id_map,
        wire_representation::{Game, Position},
    };
    fn board(
        ours: &[(i32, i32)],
        health: i32,
        opponent: &[(i32, i32)],
        opp_health: i32,
        food: &[(i32, i32)],
        hazards: &[(i32, i32)],
    ) -> (CellBoard4Snakes11x11, SnakeId) {
        let mut game: Game =
            serde_json::from_str(include_str!("../fixtures/turn33-food.json")).unwrap();
        let mut us = game.you.clone();
        us.body = ours.iter().map(|&(x, y)| Position::new(x, y)).collect();
        us.head = us.body[0];
        us.health = health;
        us.actual_length = None;
        let mut other = us.clone();
        other.id = "race-opponent".into();
        other.body = opponent.iter().map(|&(x, y)| Position::new(x, y)).collect();
        other.head = other.body[0];
        other.health = opp_health;
        game.board.snakes = vec![us.clone(), other];
        game.you = us;
        game.board.food = food.iter().map(|&(x, y)| Position::new(x, y)).collect();
        game.board.hazards = hazards.iter().map(|&(x, y)| Position::new(x, y)).collect();
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        (game.as_cell_board(&ids).unwrap(), you)
    }
    fn inspect(board: &CellBoard4Snakes11x11, you: SnakeId) -> Assessment {
        assess(
            board,
            you,
            board.reasonable_move_mask(board.get_head_as_native_position(&you)),
            None,
            MAX_EXPANSIONS,
        )
    }
    fn food(a: &Assessment, x: u8, y: u8) -> &FoodRace {
        a.foods.iter().find(|f| f.cell == x + 11 * y).unwrap()
    }

    #[test]
    fn longer_or_equal_rival_loses_the_target_before_heads_are_adjacent() {
        for rival in [
            &[(7, 3), (7, 2), (7, 1)][..],
            &[(7, 3), (7, 2), (7, 1), (7, 0)][..],
        ] {
            let (b, y) = board(
                &[(3, 3), (3, 2), (3, 1)],
                50,
                rival,
                90,
                &[(5, 3), (2, 5)],
                &[],
            );
            let a = inspect(&b, y);
            assert!(!a.budget_exhausted);
            assert_eq!(food(&a, 5, 3).class, RaceClass::LikelyLost);
            assert_eq!(food(&a, 5, 3).arrival, Some(2));
            assert_eq!(food(&a, 5, 3).rival_arrival, Some(2));
            assert_eq!(a.target(None).unwrap().cell, 2 + 11 * 5);
            let (w, _) = a.priors([10; 4], 50, None).unwrap();
            assert!(w[Move::Up.as_index()] > w[Move::Right.as_index()]);
        }
    }
    #[test]
    fn leaf_scoring_discounts_lost_food_and_prefers_an_attainable_alternative() {
        use crate::eval::evaluate_board_with_food_races as eval;
        let ours = &[(3, 3), (3, 2), (3, 1)];
        let rival = &[(7, 3), (7, 2), (7, 1), (7, 0)];
        let delta = |foods: &[(i32, i32)]| {
            let (b, y) = board(ours, 50, rival, 90, foods, &[]);
            i32::from(eval(&b, &y, true)) - i32::from(eval(&b, &y, false))
        };
        assert_eq!(delta(&[(5, 3)]), -8 * 12);
        // The three-step alternative has a two-step uncertainty discount:
        // the rival may first eat the other meal. Five beats the lost race
        // (effective ten steps), without treating the alternative as secured.
        assert_eq!(delta(&[(5, 3), (2, 5)]), -3 * 12);
        let (b, y) = board(ours, 50, &rival[..2], 90, &[(5, 3)], &[]);
        assert_eq!(eval(&b, &y, true), eval(&b, &y, false));
        let (empty, y) = board(ours, 50, rival, 90, &[], &[]);
        assert_eq!(eval(&empty, &y, true), eval(&empty, &y, false));
        let (dead, y) = board(ours, 0, rival, 90, &[(5, 3)], &[]);
        assert_eq!(eval(&dead, &y, true), 0);
    }

    #[test]
    fn shorter_or_starving_rival_does_not_own_the_meal() {
        for (rival, hp) in [
            (&[(7, 3), (7, 2)][..], 90),
            (&[(7, 3), (7, 2), (7, 1), (7, 0)][..], 1),
        ] {
            let (b, y) = board(&[(3, 3), (3, 2), (3, 1)], 50, rival, hp, &[(5, 3)], &[]);
            assert_eq!(food(&inspect(&b, y), 5, 3).class, RaceClass::Favourable);
        }
    }

    #[test]
    fn blocked_rivals_and_growth_do_not_create_false_equal_arrival_wins() {
        let (b, y) = board(
            &[(3, 3), (3, 2), (3, 1)],
            50,
            &[(10, 10), (9, 10), (9, 9), (10, 9), (10, 8)],
            90,
            &[(5, 3)],
            &[],
        );
        let a = inspect(&b, y);
        assert_eq!(food(&a, 5, 3).rival, None);
        assert_eq!(food(&a, 5, 3).class, RaceClass::Favourable);
        let (b, y) = board(
            &[(3, 3), (3, 2), (3, 1)],
            50,
            &[(7, 3), (7, 2)],
            90,
            &[(5, 3), (6, 3)],
            &[],
        );
        let a = inspect(&b, y);
        assert_eq!(food(&a, 5, 3).rival_length, Some(4));
        assert_eq!(food(&a, 5, 3).class, RaceClass::Contested);
    }
    #[test]
    fn last_health_step_and_food_in_a_hazard_match_the_simulator() {
        for hazards in [&[][..], &[(4, 3)][..]] {
            let (b, y) = board(
                &[(3, 3), (3, 2), (3, 1)],
                1,
                &[(9, 9), (9, 8)],
                90,
                &[(4, 3)],
                hazards,
            );
            let a = inspect(&b, y);
            let f = food(&a, 4, 3);
            assert_eq!(f.arrival, Some(1));
            assert_eq!(f.health_margin, 0);
            let n = b
                .simulate_single_action(&[(y, Move::Right), (SnakeId(1), Move::Left)])
                .1;
            assert_eq!(n.get_health(&y), 100);
            assert_eq!(n.get_length(&y), 4);
        }
    }
    #[test]
    fn hazards_require_a_detour_and_intermediate_growth_is_uncertain() {
        let (b, y) = board(
            &[(3, 3), (3, 2), (3, 1)],
            5,
            &[(9, 9), (9, 8)],
            90,
            &[(5, 3)],
            &[(4, 3)],
        );
        assert_eq!(food(&inspect(&b, y), 5, 3).arrival, Some(4));
        let (b, y) = board(
            &[(3, 3), (3, 2), (3, 1)],
            90,
            &[(9, 9), (9, 8)],
            90,
            &[(4, 3), (5, 3)],
            &[],
        );
        let a = inspect(&b, y);
        assert!(food(&a, 5, 3).uncertain_route);
        assert_eq!(food(&a, 5, 3).class, RaceClass::Contested);
    }
    #[test]
    fn tails_are_uncertain_and_stacked_tails_are_not_immediately_free() {
        let (b, y) = board(
            &[(3, 3), (3, 2), (2, 2), (2, 3)],
            90,
            &[(9, 9), (9, 8)],
            90,
            &[(1, 3)],
            &[],
        );
        let a = inspect(&b, y);
        assert_eq!(food(&a, 1, 3).arrival, Some(2));
        assert!(food(&a, 1, 3).uncertain_route);
        let (b, y) = board(
            &[(3, 3), (3, 2), (2, 2), (2, 3), (2, 3)],
            90,
            &[(9, 9), (9, 8)],
            90,
            &[(1, 3)],
            &[],
        );
        assert!(food(&inspect(&b, y), 1, 3).arrival.unwrap() > 2);
    }
    #[test]
    fn all_adverse_races_retain_a_best_chance_and_limits_are_unknown() {
        let (b, y) = board(
            &[(3, 3), (3, 2), (3, 1)],
            20,
            &[(7, 3), (7, 2), (7, 1), (7, 0)],
            90,
            &[(5, 3)],
            &[],
        );
        let a = inspect(&b, y);
        assert!(a.best_chance);
        assert!(a.target(None).is_some());
        let mask = b.reasonable_move_mask(b.get_head_as_native_position(&y));
        for (deadline, cap) in [(None, 0), (Some(Instant::now()), MAX_EXPANSIONS)] {
            let a = assess(&b, y, mask, deadline, cap);
            assert!(a.budget_exhausted);
            assert!(a.priors([10; 4], 20, None).is_none());
        }
        let old = crate::mcts::Node::new_root_with_food_races(b, false)
            .root_candidates(y)
            .unwrap();
        let new = crate::mcts::Node::new_root_with_food_races(b, true)
            .root_candidates(y)
            .unwrap();
        assert_eq!(old.0, new.0);
        let (b, y) = board(
            &[(3, 3), (3, 2), (3, 1)],
            1,
            &[(9, 9), (9, 8)],
            90,
            &[(5, 3)],
            &[],
        );
        assert_eq!(food(&inspect(&b, y), 5, 3).class, RaceClass::BeyondHealth);
    }
    #[test]
    fn target_history_expires_and_cannot_be_resurrected_by_old_requests() {
        let mut h = TargetHistory::default();
        h.observe(10, Some(33));
        assert_eq!(h.hint(11), Some(33));
        assert_eq!(h.hint(14), None);
        h.observe(12, None);
        h.observe(9, Some(44));
        assert_eq!(h.hint(13), None);
    }
    #[test]
    fn recorded_longer_opponent_race_prefers_an_alternative_meal() {
        let games: Vec<Game> =
            serde_json::from_str(include_str!("../fixtures/food-race-e6bcd8f8.json")).unwrap();
        let g = &games[1];
        assert_eq!(g.turn, 276);
        let ids = build_snake_id_map(g);
        let y = ids[&g.you.id];
        let b = g.as_cell_board(&ids).unwrap();
        let root = crate::mcts::Node::new_root_with_food_races(b, true);
        let a = root.root_food_race(y).unwrap();
        assert_eq!(food(a, 8, 1).class, RaceClass::LikelyLost);
        assert_ne!(root.food_target(), Some(8 + 11));
        assert!(root.food_target().is_some());
    }
}
