//! Frozen pre-cleanup scoring variants for offline experiments only.
#![allow(dead_code)]

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
    let food = cellboard.get_all_food_as_positions();
    evaluate_board_impl(
        cellboard,
        you,
        length_weight,
        food_distance_weight,
        None,
        &food,
    )
}

/// Offline weighted scoring using food already tracked by the search.
pub fn evaluate_board_with_food_and_weights(
    board: &CellBoard4Snakes11x11,
    you: &SnakeId,
    food: &[Position],
    length_weight: i32,
    food_distance_weight: i32,
) -> u16 {
    evaluate_board_impl(board, you, length_weight, food_distance_weight, None, food)
}

/// Evaluate a board with the legacy low-health food term while varying the score
/// assigned to each unit of snake length. Retained for offline baseline comparisons.
pub fn evaluate_board_with_length_weight(
    cellboard: &CellBoard4Snakes11x11,
    you: &SnakeId,
    length_weight: i32,
) -> u16 {
    // Only a starving snake reads the food list here, so skip the scan for a healthy one.
    let food = if cellboard.get_health(you) < 40 {
        cellboard.get_all_food_as_positions()
    } else {
        arrayvec::ArrayVec::new()
    };
    evaluate_board_impl(cellboard, you, length_weight, 5, Some(40), &food)
}

/// [`evaluate_board`], with the board's food positions supplied by the caller.
///
/// Scoring a leaf needs only the distance to the nearest food, but `get_all_food_as_positions`
/// walks every cell of the board to build the list. A rollout already tracks exactly which cells
/// hold food, because it has to keep the list correct as food is eaten, so passing it in removes
/// a full board scan from every leaf evaluation. Passing the board's own list is equivalent, which
/// `supplied_food_list_matches_the_board_scan` pins.
pub fn evaluate_board_with_food(
    cellboard: &CellBoard4Snakes11x11,
    you: &SnakeId,
    food_positions: &[Position],
) -> u16 {
    evaluate_board_impl(cellboard, you, 3, 12, None, food_positions)
}

fn evaluate_board_impl(
    cellboard: &CellBoard4Snakes11x11,
    you: &SnakeId,
    length_weight: i32,
    food_distance_weight: i32,
    food_health_threshold: Option<u8>,
    food_positions: &[Position],
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
    // `free_neighbor_count` is the same count as `free_neighbors(..).count()` for every cell, but
    // it works from cell indices instead of building a `Position` and a `Vector` per neighbour.
    let immediate_moves = i32::from(cellboard.free_neighbor_count(head_native));
    score += immediate_moves * 25; // This is our proxy for area control

    let head_pos = cellboard.get_head_as_position(you);

    // 4. Food distance. The production scorer keeps this active at every health level.
    if food_health_threshold.is_none_or(|threshold| health < threshold)
        && !food_positions.is_empty()
    {
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

    // 5. Opponent awareness - avoid dangerous head-to-head collisions
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
