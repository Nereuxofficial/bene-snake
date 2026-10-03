//! Normalize Arena snapshots before their overlapping corpse bodies reach the cell board.
use battlesnake_game_types::{
    types::{Move, SnakeIDMap, SnakeId},
    wire_representation::{BattleSnake, Game, Position},
};
use color_eyre::eyre::{Result, ensure};
use std::{collections::BTreeMap, time::Instant};

pub struct GameState {
    pub ids: SnakeIDMap,
    last: Option<(i32, BTreeMap<String, Position>)>,
    dead: BTreeMap<String, i32>,
    pub touched: Instant,
}

impl GameState {
    pub fn new(game: &Game) -> Self {
        Self {
            ids: std::iter::once((game.you.id.clone(), SnakeId(0)))
                .chain(
                    game.board
                        .snakes
                        .iter()
                        .filter(|s| s.id != game.you.id)
                        .take(3)
                        .enumerate()
                        .map(|(i, s)| (s.id.clone(), SnakeId((i + 1) as u8))),
                )
                .collect(),
            last: None,
            dead: BTreeMap::new(),
            touched: Instant::now(),
        }
    }

    pub fn normalize(&mut self, game: &mut Game) -> Result<()> {
        self.touched = Instant::now();
        let mut dead: BTreeMap<String, i32> = self
            .dead
            .iter()
            .filter(|(_, turn)| **turn <= game.turn)
            .map(|(id, turn)| (id.clone(), *turn))
            .collect();
        let consecutive = self
            .last
            .as_ref()
            .filter(|(turn, _)| turn.checked_add(1) == Some(game.turn));
        // These predicates are unambiguous in a completed standard-board snapshot.
        // Wrapped boards can legitimately wrap across edges (and tiny boards can stay put).
        for snake in &game.board.snakes {
            let frozen = !game.is_wrapped()
                && consecutive.is_some_and(|(_, heads)| heads.get(&snake.id) == Some(&snake.head));
            let wall = !game.is_wrapped() && game.off_board(snake.head);
            let self_collision = snake.body.iter().skip(1).any(|p| *p == snake.head)
                && !snake.body.iter().all(|p| *p == snake.head);
            if snake.health <= 0 || frozen || wall || self_collision {
                dead.insert(snake.id.clone(), game.turn);
            }
        }
        // Remove old corpses first. Compare current heads/body collisions as a batch:
        // removing a loser while iterating would hide tied head collisions.
        let remaining: Vec<_> = game
            .board
            .snakes
            .iter()
            .filter(|s| !dead.contains_key(&s.id))
            .collect();
        let collisions: Vec<_> = remaining
            .iter()
            .filter(|snake| {
                remaining.iter().any(|other| {
                    other.id != snake.id
                        && ((other.head == snake.head && other.body.len() >= snake.body.len())
                            || other.body.iter().skip(1).any(|p| *p == snake.head))
                })
            })
            .map(|s| s.id.clone())
            .collect();
        // With no consecutive history, a late /move can contain an old corpse
        // underneath a live head. Reject the ambiguous overlap until the next
        // observation rather than permanently declaring the live snake dead.
        if consecutive.is_some() || game.turn == 0 {
            for id in collisions {
                dead.insert(id, game.turn);
            }
        }
        let heads = game
            .board
            .snakes
            .iter()
            .map(|s| (s.id.clone(), s.head))
            .collect();
        game.board.snakes.retain(|s| !dead.contains_key(&s.id));
        if let Some(you) = game.board.snakes.iter().find(|s| s.id == game.you.id) {
            game.you = you.clone();
        }
        // Keep raw observations even when overlapping corpses make the first
        // mid-game snapshot ambiguous. The next consecutive turn can identify
        // frozen opponents; inferred deaths are only committed after validation.
        let newer = self.last.as_ref().is_none_or(|(turn, _)| game.turn > *turn);
        if newer {
            self.last = Some((game.turn, heads));
        }
        validate(game)?;
        // A missing /start can omit already-dead IDs; newly observed live IDs still
        // need stable slots. Never renumber an existing snake.
        for snake in &game.board.snakes {
            if !self.ids.contains_key(&snake.id) {
                let slot = (1..4).find(|i| !self.ids.values().any(|id| id.0 == *i));
                let slot =
                    slot.ok_or_else(|| color_eyre::eyre::eyre!("no compact snake ID slot"))?;
                self.ids.insert(
                    snake.id.clone(),
                    battlesnake_game_types::types::SnakeId(slot),
                );
            }
        }
        if newer {
            for (id, turn) in &dead {
                self.dead.entry(id.clone()).or_insert(*turn);
            }
        }
        Ok(())
    }
}

fn validate(game: &Game) -> Result<()> {
    ensure!(
        game.board.width > 0 && game.board.height > 0,
        "empty board dimensions"
    );
    ensure!(
        game.board.snakes.iter().any(|s| s.id == game.you.id),
        "our snake is absent or eliminated"
    );
    let mut occupied = BTreeMap::new();
    let mut ids = std::collections::BTreeSet::new();
    for snake in &game.board.snakes {
        ensure!(ids.insert(&snake.id), "duplicate snake ID");
        ensure!((1..=100).contains(&snake.health), "invalid living health");
        ensure!(
            snake.body.front() == Some(&snake.head),
            "head does not match body"
        );
        let stacked_start = snake.body.len() <= 3 && snake.body.iter().all(|p| *p == snake.head);
        let mut positions = std::collections::BTreeSet::new();
        for (i, p) in snake.body.iter().enumerate() {
            ensure!(!game.off_board(*p), "body outside board");
            if i > 0 {
                let prev = snake.body[i - 1];
                let dx = (i64::from(prev.x) - i64::from(p.x)).abs();
                let dy = (i64::from(prev.y) - i64::from(p.y)).abs();
                let distance = if game.is_wrapped() {
                    dx.min(i64::from(game.board.width) - dx)
                        + dy.min(i64::from(game.board.height) - dy)
                } else {
                    dx + dy
                };
                ensure!(distance <= 1, "disconnected body segments");
            }
            let key = (p.x, p.y);
            let doubled_tail = i + 1 == snake.body.len() && i > 0 && snake.body[i - 1] == *p;
            ensure!(
                positions.insert(key) || stacked_start || doubled_tail,
                "unsupported body overlap"
            );
            if let Some(owner) = occupied.insert(key, &snake.id) {
                ensure!(owner == &snake.id, "overlapping live bodies");
            }
        }
    }
    Ok(())
}

fn destination(game: &Game, head: Position, mv: Move) -> Position {
    let p = head.add_vec(mv.to_vector());
    if game.is_wrapped() && game.board.width > 0 && game.board.height > 0 {
        Position::new(
            p.x.rem_euclid(game.board.width as i32),
            p.y.rem_euclid(game.board.height as i32),
        )
    } else {
        p
    }
}

fn physical_moves(game: &Game, snake: &BattleSnake) -> [bool; 4] {
    let mut moves = [false; 4];
    for mv in Move::all() {
        let p = destination(game, snake.head, mv);
        let food = game.board.food.contains(&p);
        let damage = if game.board.hazards.contains(&p) {
            game.game
                .ruleset
                .settings
                .as_ref()
                .map_or(15, |s| s.hazard_damage_per_turn.max(0))
        } else {
            0
        };
        if snake.body.get(1) == Some(&p)
            || game.off_board(p)
            || (!food && i64::from(snake.health) <= 1 + i64::from(damage))
        {
            continue;
        }
        let blocked = game
            .board
            .snakes
            .iter()
            .chain(
                std::iter::once(snake)
                    .filter(|s| !game.board.snakes.iter().any(|other| other.id == s.id)),
            )
            .any(|other| {
                // Standard movement pops the old tail before feeding duplicates the new tail.
                // A stacked tail remains occupied by its preceding segment.
                other
                    .body
                    .iter()
                    .take(other.body.len().saturating_sub(1))
                    .any(|cell| *cell == p)
            });
        if !blocked {
            moves[mv.as_index()] = true;
        }
    }
    moves
}

pub struct ResponseMoves {
    pub fallback: Move,
    pub acceptable: [bool; 4],
}

pub fn response_moves(game: &Game) -> ResponseMoves {
    let physical = physical_moves(game, &game.you);
    let mut safe = physical;
    for other in &game.board.snakes {
        if other.id == game.you.id || other.health <= 0 || other.body.len() < game.you.body.len() {
            continue;
        }
        let replies = physical_moves(game, other);
        for mv in Move::all() {
            if Move::all().iter().any(|reply| {
                replies[reply.as_index()]
                    && destination(game, other.head, *reply) == destination(game, game.you.head, mv)
            }) {
                safe[mv.as_index()] = false;
            }
        }
    }
    let acceptable = if safe.iter().any(|v| *v) {
        safe
    } else {
        physical
    };
    let fallback = Move::all()
        .iter()
        .copied()
        .find(|m| acceptable[m.as_index()])
        .unwrap_or(Move::Up);
    ResponseMoves {
        fallback,
        acceptable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use battlesnake_game_types::{
        compact_representation::standard::CellBoard4Snakes11x11,
        types::{HealthGettableGame, ReasonableMovesGame, SimulableGame},
    };

    #[test]
    fn captured_death_sequences_match_replay_and_convert() {
        let sequences: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(include_str!("fixtures/arena-death-sequences.json")).unwrap();
        for sequence in sequences {
            let first: Game = serde_json::from_value(sequence[0]["request"].clone()).unwrap();
            let mut state = GameState::new(&first);
            for case in sequence {
                let mut game: Game = serde_json::from_value(case["request"].clone()).unwrap();
                state
                    .normalize(&mut game)
                    .unwrap_or_else(|e| panic!("{} turn {}: {e}", game.game.id, game.turn));
                let mut living: Vec<_> = game.board.snakes.iter().map(|s| s.id.clone()).collect();
                living.sort();
                let expected: Vec<String> =
                    serde_json::from_value(case["living_ids"].clone()).unwrap();
                assert_eq!(living, expected, "{} turn {}", game.game.id, game.turn);
                let _: CellBoard4Snakes11x11 = game.as_cell_board(&state.ids).unwrap();
                let mut retry: Game = serde_json::from_value(case["request"].clone()).unwrap();
                state.normalize(&mut retry).unwrap();
                assert_eq!(game.board.snakes, retry.board.snakes);
            }
            // Replaying the opening after later deaths must not remove those opponents.
            let mut old = first.clone();
            state.normalize(&mut old).unwrap();
            assert_eq!(old.board.snakes, first.board.snakes);
        }
    }

    fn game_with_bodies(bodies: &[&[(i32, i32)]]) -> Game {
        let mut game: Game =
            serde_json::from_str(include_str!("../lib/fixtures/turn33-food.json")).unwrap();
        let base = game.you.clone();
        game.board.food.clear();
        game.board.hazards.clear();
        game.board.snakes = bodies
            .iter()
            .enumerate()
            .map(|(i, body)| {
                let mut snake = base.clone();
                snake.id = format!("snake-{i}");
                snake.health = 90;
                snake.body = body.iter().map(|(x, y)| Position::new(*x, *y)).collect();
                snake.head = snake.body[0];
                snake
            })
            .collect();
        game.you = game.board.snakes[0].clone();
        game.turn = 0;
        game
    }

    #[test]
    fn stacked_start_and_growth_tail_are_alive() {
        let mut game = game_with_bodies(&[&[(1, 1), (1, 1), (1, 1)], &[(8, 8), (8, 7), (8, 7)]]);
        GameState::new(&game).normalize(&mut game).unwrap();
        assert_eq!(game.board.snakes.len(), 2);
    }

    #[test]
    fn frozen_tracking_requires_consecutive_turns_and_ignores_wrapped() {
        let mut game = game_with_bodies(&[&[(1, 1), (1, 0), (0, 0)], &[(8, 8), (8, 7), (8, 6)]]);
        let mut state = GameState::new(&game);
        state.normalize(&mut game).unwrap();
        game.turn = 2;
        state.normalize(&mut game).unwrap(); // A gap is insufficient evidence.
        assert_eq!(game.board.snakes.len(), 2);
        game.game.ruleset.name = "wrapped".into();
        game.turn = 3;
        state.normalize(&mut game).unwrap();
        assert_eq!(game.board.snakes.len(), 2);
    }

    #[test]
    fn fallback_avoids_neck_wall_and_losing_head_contest() {
        let game = game_with_bodies(&[&[(0, 1), (0, 0), (1, 0)], &[(2, 1), (2, 2), (2, 3)]]);
        let moves = response_moves(&game);
        assert_eq!(moves.fallback, Move::Up);
        assert!(moves.acceptable[Move::Up.as_index()]);
        assert!(!moves.acceptable[Move::Down.as_index()]);
        assert!(!moves.acceptable[Move::Left.as_index()]);
        assert!(!moves.acceptable[Move::Right.as_index()]);
    }

    #[test]
    fn fallback_can_enter_a_vacating_tail_even_when_its_owner_eats() {
        let mut game = game_with_bodies(&[&[(1, 1), (1, 0), (0, 0)], &[(2, 3), (2, 2), (2, 1)]]);
        assert!(response_moves(&game).acceptable[Move::Right.as_index()]);
        game.board.food.push(Position::new(3, 3));
        assert!(response_moves(&game).acceptable[Move::Right.as_index()]);
        game.board.food.clear();
        game.board.snakes[1].body.push_back(Position::new(2, 1));
        assert!(!response_moves(&game).acceptable[Move::Right.as_index()]);
    }

    #[test]
    fn captured_arena_tail_entries_match_simulated_survival() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("fixtures/tail-entry-arena.json")).unwrap();
        for case in cases {
            let mut game: Game = serde_json::from_value(case["request"].clone()).unwrap();
            let mut state = GameState::new(&game);
            state.normalize(&mut game).unwrap();
            let escape = Move::all()
                .into_iter()
                .find(|mv| serde_json::to_value(mv).unwrap() == case["escape"])
                .unwrap();
            let response = response_moves(&game);
            assert!(response.acceptable[escape.as_index()], "{}", game.game.id);
            assert_eq!(response.fallback, escape);
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&state.ids).unwrap();
            let moves = board.reasonable_moves_for_each_snake();
            let replies: Vec<_> = moves
                .iter()
                .map(|(id, moves)| {
                    (
                        *id,
                        if *id == SnakeId(0) {
                            vec![escape]
                        } else {
                            moves.to_vec()
                        },
                    )
                })
                .collect();
            for (_, after) in board.simulate_with_moves(&replies) {
                assert!(after.get_health(&SnakeId(0)) > 0, "{}", game.game.id);
            }
        }
    }

    #[test]
    fn starvation_and_lethal_hazards_are_rejected_but_food_rescues() {
        let mut game = game_with_bodies(&[&[(1, 1), (1, 0), (0, 0)]]);
        game.you.health = 1;
        game.board.snakes[0].health = 1;
        assert!(!response_moves(&game).acceptable.iter().any(|v| *v));
        game.board.food.push(Position::new(1, 2));
        assert_eq!(response_moves(&game).fallback, Move::Up);
        game.board.food.clear();
        game.you.health = 10;
        game.board.snakes[0].health = 10;
        game.board.hazards.push(Position::new(1, 2));
        assert!(!response_moves(&game).acceptable[Move::Up.as_index()]);
    }
    #[test]
    fn missing_start_with_ambiguous_overlap_recovers_on_next_turn() {
        let mut game = game_with_bodies(&[
            &[(1, 1), (1, 0), (0, 0)],
            &[(4, 0), (3, 0), (2, 0)],
            &[(5, 0), (4, 0), (3, 0)],
        ]);
        game.turn = 20;
        let mut state = GameState::new(&game);
        assert!(state.normalize(&mut game.clone()).is_err()); // Body/body overlap cannot identify a corpse yet.
        game.turn += 1;
        game.board.snakes[0].body = [
            Position::new(1, 2),
            Position::new(1, 1),
            Position::new(1, 0),
        ]
        .into();
        game.board.snakes[0].head = Position::new(1, 2);
        game.board.snakes[1].body = [
            Position::new(4, 1),
            Position::new(4, 0),
            Position::new(3, 0),
        ]
        .into();
        game.board.snakes[1].head = Position::new(4, 1);
        state.normalize(&mut game).unwrap();
        assert_eq!(game.board.snakes.len(), 2);
        assert!(game.board.snakes.iter().all(|s| s.id != "snake-2"));
    }

    #[test]
    fn fallback_still_checks_our_body_when_normalization_excludes_us() {
        let mut game = game_with_bodies(&[&[(1, 1), (1, 0), (0, 0), (0, 1)]]);
        game.board.snakes.clear();
        let moves = response_moves(&game);
        assert!(!moves.acceptable[Move::Down.as_index()]);
        // The tail can vacate; turning into it remains permitted.
        assert!(moves.acceptable[Move::Left.as_index()]);
    }
}
