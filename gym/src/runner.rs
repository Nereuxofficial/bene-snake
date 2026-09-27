use std::collections::VecDeque;

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        ReasonableMovesGame, SimulableGame, StandardFoodPlaceableGame, VictorDeterminableGame,
        build_snake_id_map,
    },
    wire_representation::{BattleSnake, Board, Game, NestedGame, Position, Ruleset},
};
use rand::seq::SliceRandom;
use rand::{Rng, RngExt, SeedableRng, rngs::SmallRng};

use crate::stats::GameResult;
use lib::Agent;

/// Configuration for game generation
#[derive(Clone, Debug)]
pub struct GameConfig {
    pub width: u32,
    pub height: u32,
    pub num_snakes: usize,
    pub initial_health: i32,
    pub initial_length: usize,
    pub num_food: usize,
    pub max_turns: u32,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            width: 11,
            height: 11,
            num_snakes: 4,
            initial_health: 100,
            initial_length: 3,
            num_food: 5,
            max_turns: 500,
        }
    }
}

impl GameConfig {
    pub fn standard_4_snake() -> Self {
        Self::default()
    }

    pub fn duel() -> Self {
        Self {
            num_snakes: 2,
            num_food: 3,
            ..Default::default()
        }
    }
}

/// Generates a random starting position for the game
pub fn generate_random_game(config: &GameConfig) -> Game {
    let mut rng = rand::rng();
    generate_random_game_with_rng(config, &mut rng)
}

pub fn generate_random_game_with_seed(config: &GameConfig, seed: u64) -> Game {
    let mut rng = SmallRng::seed_from_u64(seed);
    generate_random_game_with_rng(config, &mut rng)
}

fn generate_random_game_with_rng(config: &GameConfig, rng: &mut impl Rng) -> Game {
    // Standard starting positions for snakes (corners and edges)
    let standard_positions = vec![
        Position::new(1, 1),
        Position::new(1, 5),
        Position::new(1, 9),
        Position::new(5, 1),
        Position::new(5, 9),
        Position::new(9, 1),
        Position::new(9, 5),
        Position::new(9, 9),
    ];

    // Shuffle and take positions for snakes
    let mut positions = standard_positions;
    positions.shuffle(rng);
    let snake_positions: Vec<_> = positions.into_iter().take(config.num_snakes).collect();

    // Create snakes
    let snakes: Vec<BattleSnake> = snake_positions
        .iter()
        .enumerate()
        .map(|(i, pos)| {
            let mut body = VecDeque::new();
            // Initial body: head at pos, rest of body stacked at the same position
            body.push_back(*pos);
            for _ in 1..config.initial_length {
                body.push_back(*pos);
            }

            BattleSnake {
                id: format!("snake_{}", i),
                name: format!("Snake {}", i),
                head: *pos,
                body,
                health: config.initial_health,
                shout: None,
                actual_length: None,
            }
        })
        .collect();

    // Generate food positions (avoid snake positions)
    let mut food = Vec::new();
    let occupied: std::collections::HashSet<_> = snake_positions.iter().collect();

    while food.len() < config.num_food {
        let pos = Position::new(
            rng.random_range(0..config.width as i32),
            rng.random_range(0..config.height as i32),
        );
        if !occupied.contains(&pos) && !food.contains(&pos) {
            food.push(pos);
        }
    }

    let board = Board {
        height: config.height,
        width: config.width,
        food,
        snakes: snakes.clone(),
        hazards: vec![],
    };

    Game {
        you: snakes[0].clone(),
        board,
        turn: 0,
        latency: 0,
        timeout: 500,
        game: NestedGame {
            id: "gym-game".to_string(),
            ruleset: Ruleset {
                name: "standard".to_string(),
                version: "v1.0.0".to_string(),
                settings: None,
            },
            timeout: 500,
            map: None,
            source: None,
        },
    }
}

/// Runs a single game with the given agents
pub fn run_game(agents: &[&dyn Agent], config: &GameConfig) -> GameResult {
    let mut rng = rand::rng();
    let game = generate_random_game_with_rng(config, &mut rng);
    run_game_from_start(agents, config, game, &mut rng)
}

pub fn run_game_seeded(agents: &[&dyn Agent], config: &GameConfig, seed: u64) -> GameResult {
    let mut rng = SmallRng::seed_from_u64(seed);
    let game = generate_random_game_with_rng(config, &mut rng);
    run_game_from_start(agents, config, game, &mut rng)
}

fn run_game_from_start(
    agents: &[&dyn Agent],
    config: &GameConfig,
    game: Game,
    rng: &mut impl Rng,
) -> GameResult {
    assert!(
        agents.len() >= config.num_snakes,
        "Need at least {} agents for {} snakes",
        config.num_snakes,
        config.num_snakes
    );

    let snake_id_map = build_snake_id_map(&game);
    let mut board: CellBoard4Snakes11x11 = game
        .as_cell_board(&snake_id_map)
        .expect("Failed to create cell board");

    let mut turn = 0;

    // Game loop
    // The compact board's `is_over` is perspective-relative: it also returns true
    // when SnakeId(0) (the API's `you`) dies. A gym game continues while at least
    // two snakes remain, regardless of which one died.
    while board.alive_snake_count() > 1 && turn < config.max_turns {
        // Legal moves are identical for every agent in this simultaneous turn;
        // compute them once instead of rescanning the board once per snake.
        let moves: Vec<_> = board
            .reasonable_moves_for_each_snake()
            .into_iter()
            .filter_map(|(snake_id, legal_moves)| {
                if legal_moves.is_empty() {
                    return None;
                }
                let agent = agents
                    .get(snake_id.0 as usize)
                    .expect("agent list must include every simulated snake");
                let mv = agent.choose_move(&board, snake_id);
                Some((snake_id, [mv]))
            })
            .collect();

        // If no moves available, game is over
        if moves.is_empty() {
            break;
        }

        // Simulate the turn
        let next_board_opt: Option<CellBoard4Snakes11x11> =
            board.simulate_with_moves(&moves).next().map(|(_, b)| b);

        if let Some(mut next_board) = next_board_opt {
            next_board.place_food(rng);
            board = next_board;
        } else {
            break;
        }

        turn += 1;
    }

    // Determine winner
    // `get_winner` also returns a survivor when SnakeId(0) has died, which is
    // useful to the live snake's perspective but is not a gym game result.
    let winner = (board.alive_snake_count() == 1)
        .then(|| board.get_winner())
        .flatten();

    GameResult {
        winner: winner.map(|w| w.0 as usize),
        turns: turn,
        num_snakes: config.num_snakes,
    }
}

/// Run multiple games and collect results
pub fn run_tournament(
    agents: &[&dyn Agent],
    config: &GameConfig,
    num_games: usize,
) -> Vec<GameResult> {
    (0..num_games).map(|_| run_game(agents, config)).collect()
}

/// Run multiple games in parallel
pub fn run_tournament_parallel(
    agents: &[&dyn Agent],
    config: &GameConfig,
    num_games: usize,
) -> Vec<GameResult> {
    use rayon::prelude::*;

    (0..num_games)
        .into_par_iter()
        .map(|_| run_game(agents, config))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use battlesnake_game_types::types::{FoodGettableGame, Move, SnakeId};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct EatThenObserve {
        calls: AtomicUsize,
        saw_replacement: AtomicBool,
    }

    impl Agent for EatThenObserve {
        fn name(&self) -> &str {
            "eat-then-observe"
        }

        fn choose_move(&self, board: &CellBoard4Snakes11x11, _you: SnakeId) -> Move {
            if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
                Move::Right
            } else {
                self.saw_replacement.store(
                    !board.get_all_food_as_positions().is_empty(),
                    Ordering::Relaxed,
                );
                Move::Up
            }
        }
    }

    struct MoveLeft;

    impl Agent for MoveLeft {
        fn name(&self) -> &str {
            "move-left"
        }

        fn choose_move(&self, _board: &CellBoard4Snakes11x11, _you: SnakeId) -> Move {
            Move::Left
        }
    }

    #[test]
    fn seeded_starts_reproduce_the_same_board() {
        let config = GameConfig::duel();
        assert_eq!(
            generate_random_game_with_seed(&config, 20260924),
            generate_random_game_with_seed(&config, 20260924)
        );
    }

    #[test]
    fn replenishes_food_after_the_last_piece_is_eaten() {
        let config = GameConfig {
            max_turns: 2,
            ..GameConfig::duel()
        };
        let mut game = generate_random_game_with_seed(&config, 20260926);
        let head = game.board.snakes[0].head;
        game.board.food = vec![Position::new(head.x + 1, head.y)];
        let eater = EatThenObserve {
            calls: AtomicUsize::new(0),
            saw_replacement: AtomicBool::new(false),
        };
        let mut rng = SmallRng::seed_from_u64(20260926);
        run_game_from_start(&[&eater, &MoveLeft], &config, game, &mut rng);
        assert!(eater.calls.load(Ordering::Relaxed) >= 2);
        assert!(eater.saw_replacement.load(Ordering::Relaxed));
    }

    struct FixedMove(Move);

    impl Agent for FixedMove {
        fn name(&self) -> &str {
            "fixed-move"
        }

        fn choose_move(&self, _board: &CellBoard4Snakes11x11, _you: SnakeId) -> Move {
            self.0
        }
    }

    struct CountMove {
        calls: std::sync::Arc<AtomicUsize>,
        movement: Move,
    }

    impl Agent for CountMove {
        fn name(&self) -> &str {
            "count-move"
        }

        fn choose_move(&self, _board: &CellBoard4Snakes11x11, _you: SnakeId) -> Move {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.movement
        }
    }

    #[test]
    fn two_snake_turn_cap_with_both_survivors_is_a_draw() {
        let config = GameConfig {
            max_turns: 3,
            ..GameConfig::duel()
        };
        let mut game = generate_random_game_with_seed(&config, 20260929);
        let positions = [Position::new(0, 0), Position::new(0, 10)];
        for (snake, position) in game.board.snakes.iter_mut().zip(positions) {
            snake.head = position;
            snake.body = VecDeque::from([position, position, position]);
            snake.health = 100;
        }
        game.you = game.board.snakes[0].clone();
        game.board.food = vec![];
        let mut rng = SmallRng::seed_from_u64(11);

        // Both snakes move right along their own row and stay alive to the cap.
        let result = run_game_from_start(
            &[&FixedMove(Move::Right), &FixedMove(Move::Right)],
            &config,
            game,
            &mut rng,
        );

        assert_eq!(result.turns, 3);
        assert_eq!(result.winner, None, "two survivors at the cap is a draw");
        assert_eq!(result.num_snakes, 2);
    }

    #[test]
    fn two_snake_game_with_one_death_reports_the_survivor() {
        let config = GameConfig {
            max_turns: 5,
            ..GameConfig::duel()
        };
        let mut game = generate_random_game_with_seed(&config, 20260930);
        let positions = [Position::new(0, 10), Position::new(0, 0)];
        for (snake, position) in game.board.snakes.iter_mut().zip(positions) {
            snake.head = position;
            snake.body = VecDeque::from([position, position, position]);
            snake.health = 100;
        }
        game.you = game.board.snakes[0].clone();
        game.board.food = vec![];
        let mut rng = SmallRng::seed_from_u64(12);

        // Snake 0 is on the top row and moves up off the board; snake 1 is on
        // the bottom row and moves right safely.
        let result = run_game_from_start(
            &[&FixedMove(Move::Up), &FixedMove(Move::Right)],
            &config,
            game,
            &mut rng,
        );

        assert_eq!(result.winner, Some(1));
        assert_eq!(result.turns, 1);
    }

    #[test]
    fn four_snake_gym_continues_after_snake_zero_dies() {
        let config = GameConfig {
            max_turns: 3,
            ..GameConfig::default()
        };
        let mut game = generate_random_game_with_seed(&config, 20260927);
        let positions = [
            Position::new(0, 0),
            Position::new(3, 0),
            Position::new(6, 0),
            Position::new(9, 0),
        ];
        for (snake, position) in game.board.snakes.iter_mut().zip(positions) {
            snake.head = position;
            snake.body = VecDeque::from([position, position, position]);
            snake.health = 100;
        }
        game.board.snakes[0].health = 1;
        game.you = game.board.snakes[0].clone();
        game.board.food = vec![Position::new(5, 5)];

        let survivor_calls = std::sync::Arc::new(AtomicUsize::new(0));
        // Snake zero survives the neck check but starves after moving right.
        let die = FixedMove(Move::Right);
        let survivors: Vec<_> = (0..3)
            .map(|_| CountMove {
                calls: std::sync::Arc::clone(&survivor_calls),
                movement: Move::Up,
            })
            .collect();
        let mut agents: Vec<&dyn Agent> = vec![&die];
        agents.extend(survivors.iter().map(|agent| agent as &dyn Agent));
        let mut rng = SmallRng::seed_from_u64(7);

        let result = run_game_from_start(&agents, &config, game, &mut rng);

        assert_eq!(result.turns, 3);
        assert_eq!(result.winner, None);
        assert_eq!(survivor_calls.load(Ordering::Relaxed), 9);
    }
}
