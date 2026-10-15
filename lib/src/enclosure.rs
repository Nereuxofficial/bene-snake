//! Soft, bounded exit viability at a leaf; static arrival estimates are not proofs.
use battlesnake_game_types::{
    compact_representation::{CellIndex, standard::CellBoard4Snakes11x11},
    types::{
        FoodQueryableGame, HazardQueryableGame, HeadGettableGame, HealthGettableGame,
        LengthGettableGame, NeckQueryableGame, SnakeBodyGettableGame, SnakeId,
    },
};

const MISSING: u8 = u8::MAX;
const HORIZON: u8 = 12;
/// Fixed before replay screening. No live leaf becomes a terminal result.
pub const MAX_PENALTY: i32 = 96;

#[derive(Clone, Copy, Debug, Default)]
pub struct Assessment {
    pub usable: u8,
    pub open: u8,
    pub contested_exits: u8,
    pub uncertain: bool,
    pub penalty: i32,
}

fn direction_bit(from: usize, to: u8) -> u8 {
    match i16::from(to) - from as i16 {
        11 => 1,
        -11 => 2,
        -1 => 4,
        1 => 8,
        _ => unreachable!(),
    }
}

fn neighbors(cell: u8) -> impl Iterator<Item = u8> {
    [
        (cell >= 11).then(|| cell - 11),
        (cell < 110).then(|| cell + 11),
        (!cell.is_multiple_of(11)).then(|| cell - 1),
        (cell % 11 < 10).then(|| cell + 1),
    ]
    .into_iter()
    .flatten()
}

#[derive(Clone, Copy)]
struct Paths {
    distance: [u8; 121],
    growth: [u8; 121],
    tails: [bool; 121],
}

// Each cell is enqueued once. Longer/dynamic routes are deliberately not claimed
// impossible. Health and hazards suppress infeasible static rival threats.
fn paths(board: &CellBoard4Snakes11x11, id: SnakeId, tails: &[bool; 121]) -> Paths {
    paths_with_horizon(board, id, tails, HORIZON)
}

fn paths_with_horizon(
    board: &CellBoard4Snakes11x11,
    id: SnakeId,
    tails: &[bool; 121],
    horizon: u8,
) -> Paths {
    let start = board.get_head_as_native_position(&id).as_usize();
    let first_mask = board.reasonable_move_mask(board.get_head_as_native_position(&id));
    let mut result = Paths {
        distance: [MISSING; 121],
        growth: [0; 121],
        tails: [false; 121],
    };
    let mut health = [0u8; 121];
    let mut queue = [0u8; 121];
    result.distance[start] = 0;
    health[start] = board.get_health(&id);
    queue[0] = start as u8;
    let (mut read, mut write) = (0, 1);
    while read < write {
        let cell = queue[read] as usize;
        read += 1;
        if result.distance[cell] >= horizon {
            continue;
        }
        for next in neighbors(cell as u8) {
            if cell == start {
                let bit = match i16::from(next) - cell as i16 {
                    11 => 1,
                    -11 => 2,
                    -1 => 4,
                    1 => 8,
                    _ => unreachable!(),
                };
                if first_mask & bit == 0 {
                    continue;
                }
            }
            let n = next as usize;
            let native = CellIndex::from_usize(n);
            if cell == start && board.is_neck(&id, &native) {
                continue;
            }
            if result.distance[n] != MISSING || !board.cell_is_free(native) {
                continue;
            }
            let food = board.is_food(&native);
            let cost = 1u16
                + if board.is_hazard(&native) {
                    u16::from(board.get_hazard_damage())
                } else {
                    0
                };
            let remaining = if food {
                100
            } else {
                u16::from(health[cell]).saturating_sub(cost) as u8
            };
            if remaining == 0 {
                continue;
            }
            result.distance[n] = result.distance[cell] + 1;
            result.growth[n] = result.growth[cell] + u8::from(food);
            result.tails[n] = result.tails[cell] || tails[n];
            health[n] = remaining;
            queue[write] = next;
            write += 1;
        }
    }
    result
}

fn region(
    available: &[bool; 121],
    threatened: &[bool; 121],
    start: usize,
    first_mask: u8,
) -> ([bool; 121], u8, u8) {
    let mut queue = [0u8; 121];
    let mut seen = [false; 121];
    let mut blocked_seen = [false; 121];
    queue[0] = start as u8;
    seen[start] = true;
    let (mut read, mut write) = (0, 1);
    let mut exits = 0u8;
    while read < write {
        let cell = queue[read];
        read += 1;
        for next in neighbors(cell) {
            if cell as usize == start && first_mask & direction_bit(start, next) == 0 {
                continue;
            }
            let n = next as usize;
            if !available[n] || seen[n] {
                continue;
            }
            if threatened[n] {
                if !blocked_seen[n] {
                    exits += 1;
                    blocked_seen[n] = true;
                }
                continue;
            }
            seen[n] = true;
            queue[write] = next;
            write += 1;
        }
    }
    (seen, write as u8, exits)
}

/// Penalise a shortage of space accessible before a dangerous rival can gate
/// an exit. There is no reward for winning territory, and no move is removed.
/// Work is bounded by four arrival BFS passes plus one 121-cell region pass.
pub fn assess(board: &CellBoard4Snakes11x11, you: SnakeId) -> Assessment {
    if board.get_health(&you) == 0 {
        return Assessment::default();
    }
    let mut tails = [false; 121];
    for i in 0..4 {
        let id = SnakeId(i);
        if board.get_health(&id) > 0
            && let Some(tail) = board.get_snake_body_vec(&id).last()
        {
            tails[tail.as_usize()] = true;
        }
    }
    let ours = paths_with_horizon(board, you, &tails, 120);
    let length = board.get_length(&you);
    let mut threatened = [false; 121];
    let mut uncertain = false;
    for i in 0..4 {
        let id = SnakeId(i);
        if id == you || board.get_health(&id) == 0 {
            continue;
        }
        let rival_length = board.get_length(&id);
        if rival_length.saturating_add(u16::from(HORIZON)) < length {
            continue;
        }
        let rival = paths(board, id, &tails);
        for n in 0..121 {
            // On a simultaneous arrival at the same meal, both snakes grow.
            // Preserve our existing length advantage; do not let the rival's
            // target-food growth alone turn a losing tie into a gate. Earlier
            // rival consumption and intermediate food remain soft estimates.
            let own_arrival_length = length
                + u16::from(
                    rival.distance[n] == ours.distance[n]
                        && board.is_food(&CellIndex::from_usize(n)),
                );
            if ours.distance[n] != MISSING
                && rival.distance[n] != MISSING
                && rival.distance[n] <= ours.distance[n]
                && rival_length + u16::from(rival.growth[n]) >= own_arrival_length
            {
                // Routes through tails or intermediate food have uncertain
                // timing/length; retain that uncertainty in the final weight.
                threatened[n] = true;
                uncertain |= rival.tails[n] || rival.growth[n] > 0;
            }
        }
    }
    let start = board.get_head_as_native_position(&you).as_usize();
    let available = std::array::from_fn(|n| {
        board.cell_is_free(CellIndex::from_usize(n)) && ours.distance[n] != MISSING
    });
    let first_mask = board.reasonable_move_mask(board.get_head_as_native_position(&you));
    let mut first_mask = first_mask;
    for n in neighbors(start as u8) {
        if board.is_neck(&you, &CellIndex::from_usize(n as usize)) {
            first_mask &= !direction_bit(start, n);
        }
    }
    let (seen, usable, exits) = region(&available, &threatened, start, first_mask);
    for n in 0..121 {
        if seen[n] {
            uncertain |= ours.growth[n] > 0 || ours.tails[n];
        }
    }
    // A nearby releasing own segment can reopen the static region. Food can
    // delay that release; discount rather than certify the region as closed.
    let body = board.get_snake_body_vec(&you);
    for (index, segment) in body.iter().enumerate().skip(1) {
        if body.len() - index <= HORIZON as usize
            && neighbors(segment.as_usize() as u8).any(|n| seen[n as usize])
        {
            uncertain = true;
        }
    }
    let open = ours.distance.iter().filter(|&&d| d != MISSING).count() as u8;
    // A long snake only needs its own length here; extra open space earns no
    // bonus. Current bodies may move, so all deficits remain bounded heuristics.
    let deficit = i32::from(length.saturating_sub(u16::from(usable)));
    let mut penalty = (deficit * 6).min(MAX_PENALTY);
    if uncertain {
        penalty /= 2;
    }
    Assessment {
        usable,
        open,
        contested_exits: exits,
        uncertain,
        penalty,
    }
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
        rival: &[(i32, i32)],
        health: i32,
        food: &[(i32, i32)],
        hazards: &[(i32, i32)],
    ) -> (CellBoard4Snakes11x11, SnakeId, SnakeId) {
        let mut g: Game =
            serde_json::from_str(include_str!("../fixtures/turn33-food.json")).unwrap();
        let mut us = g.you.clone();
        us.body = ours.iter().map(|&(x, y)| Position::new(x, y)).collect();
        us.head = us.body[0];
        us.health = 100;
        us.actual_length = None;
        let mut enemy = us.clone();
        enemy.id = "enclosure-rival".into();
        enemy.body = rival.iter().map(|&(x, y)| Position::new(x, y)).collect();
        enemy.head = enemy.body[0];
        enemy.health = health;
        g.you = us.clone();
        g.board.snakes = vec![us, enemy];
        g.board.food = food.iter().map(|&(x, y)| Position::new(x, y)).collect();
        g.board.hazards = hazards.iter().map(|&(x, y)| Position::new(x, y)).collect();
        let ids = build_snake_id_map(&g);
        (
            g.as_cell_board(&ids).unwrap(),
            ids[&g.you.id],
            ids["enclosure-rival"],
        )
    }
    #[test]
    fn opponent_gate_disconnects_a_room_without_rewarding_extra_territory() {
        let mut available = [false; 121];
        // Small room, single exit, larger room. Only the gate changes.
        for (x, y) in [
            (0, 0),
            (1, 0),
            (0, 1),
            (1, 1),
            (2, 1),
            (3, 1),
            (3, 0),
            (4, 0),
            (4, 1),
            (3, 2),
            (4, 2),
        ] {
            available[x + 11 * y] = true;
        }
        let clear = region(&available, &[false; 121], 0, 15);
        let mut threat = [false; 121];
        threat[13] = true;
        let gated = region(&available, &threat, 0, 15);
        assert_eq!(clear.1, 11);
        assert_eq!(gated.1, 4);
        assert_eq!(gated.2, 1);
    }
    #[test]
    fn arrival_paths_obey_neck_health_hazards_and_food_growth() {
        let (b, _, enemy) = board(&[(8, 8), (8, 7), (8, 6)], &[(3, 3), (3, 2)], 1, &[], &[]);
        let p = paths(&b, enemy, &[false; 121]);
        assert_eq!(p.distance[4 + 3 * 11], MISSING);
        let (b, _, enemy) = board(&[(8, 8), (8, 7), (8, 6)], &[(3, 3), (3, 2)], 100, &[], &[]);
        let p = paths(&b, enemy, &[false; 121]);
        assert_ne!(
            p.distance[3 + 2 * 11],
            1,
            "length-two neck is not a first move"
        );
        let (b, _, enemy) = board(
            &[(8, 8), (8, 7), (8, 6)],
            &[(3, 3), (3, 2)],
            2,
            &[],
            &[(4, 3)],
        );
        assert_ne!(paths(&b, enemy, &[false; 121]).distance[4 + 3 * 11], 1);
        let (b, _, enemy) = board(
            &[(8, 8), (8, 7), (8, 6)],
            &[(3, 3), (3, 2)],
            1,
            &[(4, 3)],
            &[(4, 3)],
        );
        let p = paths(&b, enemy, &[false; 121]);
        assert_eq!(p.distance[4 + 3 * 11], 1);
        assert_eq!(p.growth[4 + 3 * 11], 1);
    }
    #[test]
    fn own_lethal_hazards_do_not_count_as_access_to_a_room() {
        // Every exit from our head is a lethal hazard; a large empty room
        // beyond it must not be counted just because hazard cells are free.
        let mut g: Game =
            serde_json::from_str(include_str!("../fixtures/turn33-food.json")).unwrap();
        let mut us = g.you.clone();
        us.body = [(3, 3), (3, 2), (3, 1)]
            .map(|(x, y)| Position::new(x, y))
            .into();
        us.head = us.body[0];
        us.health = 2;
        us.actual_length = None;
        g.you = us.clone();
        g.board.snakes = vec![us];
        g.board.food.clear();
        g.board.hazards = [(3, 4), (2, 3), (4, 3)]
            .map(|(x, y)| Position::new(x, y))
            .into();
        let ids = build_snake_id_map(&g);
        let b = g.as_cell_board(&ids).unwrap();
        let a = assess(&b, ids[&g.you.id]);
        assert_eq!(a.usable, 1);
        assert!(a.penalty > 0);
    }

    #[test]
    fn shorter_rival_tying_on_food_does_not_block_our_only_first_exit() {
        // At the corner our only exit is the food at (1,0). Both heads
        // arrive in one step and both grow, so our length advantage persists.
        let (b, you, _) = board(
            &[(0, 0), (0, 1), (0, 2)],
            &[(2, 0), (2, 1)],
            100,
            &[(1, 0)],
            &[],
        );
        assert!(assess(&b, you).usable >= 2);
    }

    #[test]
    fn shorter_rival_only_gates_after_growth_and_tail_routes_are_uncertain() {
        let ours = [(6, 3), (6, 2), (6, 1)];
        let enemy = [(3, 3), (3, 2)];
        let (b, you, _) = board(&ours, &enemy, 100, &[], &[]);
        assert_eq!(assess(&b, you).contested_exits, 0);
        let (b, you, id) = board(&ours, &enemy, 100, &[(4, 3)], &[]);
        let a = assess(&b, you);
        assert!(a.contested_exits > 0);
        assert!(a.uncertain);
        let mut tails = [false; 121];
        tails[4 + 3 * 11] = true;
        assert!(paths(&b, id, &tails).tails[4 + 3 * 11]);
    }

    #[test]
    fn recent_enclosures_have_bounded_soft_scores_and_unchanged_food_baseline() {
        let games: Vec<Game> =
            serde_json::from_str(include_str!("../fixtures/enclosure-replays.json")).unwrap();
        let mut gated = 0;
        for g in games {
            let ids = build_snake_id_map(&g);
            let b = g.as_cell_board(&ids).unwrap();
            let you = ids[&g.you.id];
            let a = assess(&b, you);
            assert!((0..=MAX_PENALTY).contains(&a.penalty));
            gated += usize::from(a.contested_exits > 0);
            let baseline = crate::eval::evaluate_board_with_food_races(&b, &you, true);
            let candidate = crate::eval::evaluate_board_with_options(&b, &you, true, true);
            assert_eq!(candidate, (i32::from(baseline) - a.penalty).max(1) as u16);
        }
        assert!(gated > 0);
    }
}
