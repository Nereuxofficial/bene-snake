use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{FoodGettableGame, HeadGettableGame, HealthGettableGame, LengthGettableGame, SnakeId},
    wire_representation::Position,
};

/// Manhattan distance between two positions
fn manhattan_distance(a: &Position, b: &Position) -> i32 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

/// Lightweight evaluation function optimized for MCTS
pub fn evaluate_board(cellboard: &CellBoard4Snakes11x11, you: &SnakeId) -> u16 {
    evaluate_board_with_food_weight(cellboard, you, 3, 12)
}

/// Evaluate a board with a stronger food-distance term, independent of health.
/// This is exposed so food-seeking weights can be compared in offline experiments.
pub fn evaluate_board_with_food_weight(
    cellboard: &CellBoard4Snakes11x11,
    you: &SnakeId,
    length_weight: i32,
    food_distance_weight: i32,
) -> u16 {
    evaluate_board_impl(cellboard, you, length_weight, food_distance_weight, None)
}

/// Evaluate a board with the legacy low-health food term while varying the score
/// assigned to each unit of snake length. Retained for offline baseline comparisons.
pub fn evaluate_board_with_length_weight(
    cellboard: &CellBoard4Snakes11x11,
    you: &SnakeId,
    length_weight: i32,
) -> u16 {
    evaluate_board_impl(cellboard, you, length_weight, 5, Some(40))
}

fn evaluate_board_impl(
    cellboard: &CellBoard4Snakes11x11,
    you: &SnakeId,
    length_weight: i32,
    food_distance_weight: i32,
    food_health_threshold: Option<u8>,
) -> u16 {
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
    score += my_length * length_weight;

    // 3. Immediate mobility (number of valid moves from head) - fast approximation of space
    let head_native = cellboard.get_head_as_native_position(you);
    let immediate_moves = cellboard.free_neighbors(head_native).count() as i32;
    score += immediate_moves * 25; // This is our proxy for area control

    // 4. Food distance. The production scorer keeps this active at every health level.
    if food_health_threshold.is_none_or(|threshold| health < threshold) {
        let head_pos = cellboard.get_head_as_position(you);
        let food_positions = cellboard.get_all_food_as_positions();
        if !food_positions.is_empty() {
            let min_food_dist = food_positions
                .iter()
                .map(|food| manhattan_distance(&head_pos, food))
                .min()
                .unwrap_or(0);

            let weight = if food_health_threshold.is_some() && health < 20 {
                food_distance_weight * 2
            } else {
                food_distance_weight
            };
            score -= min_food_dist * weight;
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

    fn board_with_food(health: i32, food: Position) -> (CellBoard4Snakes11x11, SnakeId) {
        let mut game: Game =
            serde_json::from_str(include_str!("../fixtures/turn33-food.json")).unwrap();
        game.you.health = health;
        game.board
            .snakes
            .iter_mut()
            .find(|snake| snake.id == game.you.id)
            .unwrap()
            .health = health;
        game.board.food = vec![food];
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        (game.as_cell_board(&ids).unwrap(), you)
    }

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

    #[test]
    fn production_food_term_rewards_near_food_at_high_health() {
        let (close_board, you) = board_with_food(80, Position::new(9, 9));
        let (far_board, _) = board_with_food(80, Position::new(0, 0));

        assert_eq!(
            evaluate_board_with_length_weight(&close_board, &you, 3),
            evaluate_board_with_length_weight(&far_board, &you, 3),
            "the baseline evaluator ignores food above its low-health threshold"
        );
        assert!(
            evaluate_board(&close_board, &you) > evaluate_board(&far_board, &you),
            "the production evaluator should reward being closer to food at high health"
        );
    }

    #[test]
    fn production_food_term_is_stronger_than_baseline_when_hungry() {
        let (close_board, you) = board_with_food(10, Position::new(9, 9));
        let (far_board, _) = board_with_food(10, Position::new(0, 0));

        let production_delta = i32::from(evaluate_board(&close_board, &you))
            - i32::from(evaluate_board(&far_board, &you));
        let baseline_delta = i32::from(evaluate_board_with_length_weight(&close_board, &you, 3))
            - i32::from(evaluate_board_with_length_weight(&far_board, &you, 3));
        assert!(production_delta > baseline_delta);
    }
}
