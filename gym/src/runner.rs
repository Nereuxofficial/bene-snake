use std::collections::VecDeque;

use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        build_snake_id_map, ReasonableMovesGame, SimulableGame, SimulatorInstruments,
        SnakeId, VictorDeterminableGame,
    },
    wire_representation::{BattleSnake, Board, Game, NestedGame, Position, Ruleset},
};
use rand::seq::SliceRandom;
use rand::Rng;

use lib::Agent;
use crate::stats::GameResult;

#[derive(Debug)]
struct Instr;
impl SimulatorInstruments for Instr {
    fn observe_simulation(&self, _: std::time::Duration) {}
}

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
    positions.shuffle(&mut rng);
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
pub fn run_game(
    agents: &[&dyn Agent],
    config: &GameConfig,
) -> GameResult {
    assert!(
        agents.len() >= config.num_snakes,
        "Need at least {} agents for {} snakes",
        config.num_snakes,
        config.num_snakes
    );

    // Generate starting position
    let game = generate_random_game(config);
    let snake_id_map = build_snake_id_map(&game);
    let mut board: CellBoard4Snakes11x11 = game
        .as_cell_board(&snake_id_map)
        .expect("Failed to create cell board");

    let mut turn = 0;

    // Game loop
    while !board.is_over() && turn < config.max_turns {
        // Collect moves from all agents
        let moves: Vec<_> = (0..config.num_snakes)
            .filter_map(|i| {
                let snake_id = SnakeId(i as u8);
                // Check if snake is still alive (has reasonable moves)
                let has_moves = board
                    .reasonable_moves_for_each_snake()
                    .any(|(sid, moves)| sid == snake_id && moves.into_iter().next().is_some());

                if has_moves {
                    let mv = agents[i].choose_move(&board, snake_id);
                    Some((snake_id, [mv]))
                } else {
                    None
                }
            })
            .collect();

        // If no moves available, game is over
        if moves.is_empty() {
            break;
        }

        // Simulate the turn
        let next_board_opt: Option<CellBoard4Snakes11x11> = board
            .simulate_with_moves(&Instr, &moves)
            .next()
            .map(|(_, b)| b);

        if let Some(next_board) = next_board_opt {
            board = next_board;
        } else {
            break;
        }

        turn += 1;
    }

    // Determine winner
    let winner = board.get_winner();

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
    (0..num_games)
        .map(|_| run_game(agents, config))
        .collect()
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
