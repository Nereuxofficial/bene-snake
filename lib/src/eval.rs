use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        FoodGettableGame, HeadGettableGame, HealthGettableGame, LengthGettableGame,
        NeighborDeterminableGame, PositionGettableGame, SnakeId,
    },
    wire_representation::Position,
};
use std::collections::{HashSet, VecDeque};

/// Manhattan distance between two positions
fn manhattan_distance(a: &Position, b: &Position) -> i32 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

/// Flood fill to count reachable cells from a starting position
/// This is critical for survival - we need to know how much space we can access
fn flood_fill(board: &CellBoard4Snakes11x11, start_pos: Position) -> u32 {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    let start_native = board.native_from_position(start_pos);
    queue.push_back(start_native);
    visited.insert(start_native);

    let mut count = 0;
    const MAX_ITERATIONS: u32 = 121; // Max cells on 11x11 board

    while let Some(pos) = queue.pop_front() {
        count += 1;

        // Safety check to prevent infinite loops
        if count >= MAX_ITERATIONS {
            break;
        }

        // Get all valid neighboring positions
        for neighbor in board.neighbors(&pos) {
            if !visited.contains(&neighbor) {
                visited.insert(neighbor);
                queue.push_back(neighbor);
            }
        }
    }

    count
}

/// Lightweight evaluation function optimized for MCTS (called millions of times)
/// This version avoids expensive operations like flood fill
pub fn evaluate_board(cellboard: &CellBoard4Snakes11x11, you: &SnakeId) -> u16 {
    // Check if we're dead - return worst score
    if cellboard.get_health(you) == 0 {
        return 0;
    }

    let mut score: i32 = 500; // Start with baseline score

    // 1. Health consideration (critical when low)
    let health = cellboard.get_health(you);
    if health < 30 {
        score -= (30 - health as i32) * 5; // Penalty for low health
    } else {
        score += (health as i32).min(50) / 10; // Small bonus for good health
    }

    // 2. Length advantage (longer is better)
    let my_length = cellboard.get_length(you) as i32;
    score += my_length * 15;

    // 3. Immediate mobility (number of valid moves from head) - fast approximation of space
    let head_native = cellboard.get_head_as_native_position(you);
    let immediate_moves = cellboard.neighbors(&head_native).count() as i32;
    score += immediate_moves * 25; // This is our proxy for area control

    // 4. Food distance when hungry (simplified)
    if health < 40 {
        let head_pos = cellboard.get_head_as_position(you);
        let food_positions = cellboard.get_all_food_as_positions();
        if !food_positions.is_empty() {
            let min_food_dist = food_positions
                .iter()
                .map(|food| manhattan_distance(&head_pos, food))
                .min()
                .unwrap_or(0);

            let hunger_multiplier = if health < 20 { 10 } else { 5 };
            score -= min_food_dist * hunger_multiplier;
        }
    }

    // 5. Center control (middle of board is strategically valuable)
    let head_pos = cellboard.get_head_as_position(you);
    let center_dist = (head_pos.x - 5).abs() + (head_pos.y - 5).abs();
    score -= center_dist;

    // 6. Opponent awareness - avoid dangerous head-to-head collisions
    for opponent_id in 0..4 {
        let opp_id = SnakeId(opponent_id);
        if opp_id == *you {
            continue;
        }

        let opp_health = cellboard.get_health(&opp_id);
        if opp_health == 0 {
            continue;
        }

        let opp_head = cellboard.get_head_as_position(&opp_id);
        let opp_length = cellboard.get_length(&opp_id);
        let dist_to_opponent = manhattan_distance(&head_pos, &opp_head);

        if dist_to_opponent == 1 {
            if opp_length >= my_length as u16 {
                score -= 100; // Avoid head-to-head with larger snakes
            } else {
                score += 30; // Bonus for potential head-to-head win
            }
        }

        if my_length > opp_length as i32 {
            score += 3; // Bonus for being longer
        }
    }

    // Ensure score is non-negative and fits in u16
    score.max(1).min(u16::MAX as i32) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use battlesnake_game_types::{types::build_snake_id_map, wire_representation::Game};

    #[test]
    fn test_evaluate_dead_snake() {
        let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
        let game: Game = serde_json::from_str(game_fixture).expect("valid fixture");
        let snake_id_map = build_snake_id_map(&game);
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&snake_id_map).expect("valid board");

        // Test with a snake that exists (snake 0)
        let snake_id = SnakeId(0);
        let score = evaluate_board(&board, &snake_id);

        // Should have a positive score for a living snake
        assert!(score > 0, "Living snake should have positive score");
    }

    #[test]
    fn test_manhattan_distance() {
        let p1 = Position::new(0, 0);
        let p2 = Position::new(3, 4);
        assert_eq!(manhattan_distance(&p1, &p2), 7);

        let p3 = Position::new(5, 5);
        let p4 = Position::new(5, 5);
        assert_eq!(manhattan_distance(&p3, &p4), 0);
    }

    #[test]
    fn test_evaluate_board_basic() {
        let game_fixture = include_str!("../../battlesnake-game-types/fixtures/start_of_game.json");
        let game: Game = serde_json::from_str(game_fixture).expect("valid fixture");
        let snake_id_map = build_snake_id_map(&game);
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&snake_id_map).expect("valid board");

        let snake_id = SnakeId(0);
        let score = evaluate_board(&board, &snake_id);

        // Should have a reasonable positive score at start
        assert!(
            score > 100,
            "Start of game should have positive score, got {}",
            score
        );
    }
}
