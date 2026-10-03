use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{FoodGettableGame, HeadGettableGame, HealthGettableGame, LengthGettableGame, SnakeId},
    wire_representation::Position,
};

/// Manhattan distance between two positions
fn manhattan_distance(a: &Position, b: &Position) -> i32 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

/// Lightweight evaluation function optimized for MCTS.
pub fn evaluate_board(cellboard: &CellBoard4Snakes11x11, you: &SnakeId) -> u16 {
    let food = cellboard.get_all_food_as_positions();
    evaluate_board_with_food(cellboard, you, &food)
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
    let health = cellboard.get_health(you);
    if health == 0 {
        return 0;
    }

    let mut score: i32 = 500; // Start with baseline score

    // 1. Health consideration (critical when low)
    if health < 30 {
        score -= (30 - i32::from(health)) * 5; // Penalty for low health
    } else {
        score += i32::from(health.min(50)) / 10; // Small bonus for good health
    }

    // 2. Length advantage (longer is better)
    let my_length = cellboard.get_length(you);
    score += i32::from(my_length) * 3;

    // 3. Immediate mobility (number of valid moves from head) - fast approximation of space
    let head_native = cellboard.get_head_as_native_position(you);
    // `free_neighbor_count` is the same count as `free_neighbors(..).count()` for every cell, but
    // it works from cell indices instead of building a `Position` and a `Vector` per neighbour.
    let immediate_moves = i32::from(cellboard.free_neighbor_count(head_native));
    score += immediate_moves * 25; // This is our proxy for area control

    let head_pos = cellboard.get_head_as_position(you);

    // 4. Food distance, at every health level.
    if let Some(min_food_dist) = food_positions
        .iter()
        .map(|food| manhattan_distance(&head_pos, food))
        .min()
    {
        score -= min_food_dist * 12;
    }

    // 5. Opponent awareness - avoid dangerous head-to-head collisions
    for opponent_id in 0..4 {
        let opp_id = SnakeId(opponent_id);
        if opp_id == *you {
            continue;
        }

        if cellboard.get_health(&opp_id) == 0 {
            continue;
        }

        let opp_head = cellboard.get_head_as_position(&opp_id);
        let opp_length = cellboard.get_length(&opp_id);
        let dist_to_opponent = manhattan_distance(&head_pos, &opp_head);

        if dist_to_opponent == 1 && opp_length >= my_length {
            score -= 100; // Avoid being next to larger snakes
        }

        if my_length > opp_length {
            score += 30; // Bonus for being longer
        }
    }

    // Ensure score is non-negative and fits in u16
    score.max(1).min(u16::MAX as i32) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use battlesnake_game_types::{
        types::{Move, build_snake_id_map},
        wire_representation::Game,
    };

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
    fn translation_toward_center_does_not_change_evaluation() {
        fn translated_board(offset: i32, health: i32) -> (CellBoard4Snakes11x11, SnakeId) {
            let mut game: Game = serde_json::from_str(include_str!(
                "../../battlesnake-game-types/fixtures/start_of_game.json"
            ))
            .unwrap();
            let mut ours = game.you.clone();
            ours.health = health;
            ours.actual_length = None;
            ours.body = [(3, 3), (3, 2), (3, 1)]
                .map(|(x, y)| Position::new(x + offset, y + offset))
                .into();
            ours.head = ours.body[0];
            let mut opponent = ours.clone();
            opponent.id = "translation-opponent".into();
            opponent.body = [(6, 6), (6, 5), (6, 4)]
                .map(|(x, y)| Position::new(x + offset, y + offset))
                .into();
            opponent.head = opponent.body[0];
            game.you = ours.clone();
            game.board.snakes = vec![ours, opponent];
            game.board.hazards.clear();
            game.board.food = [(3, 5), (7, 7)]
                .map(|(x, y)| Position::new(x + offset, y + offset))
                .into();
            let ids = build_snake_id_map(&game);
            (game.as_cell_board(&ids).unwrap(), ids[&game.you.id])
        }

        // Food distances, mobility and opponent geometry are identical, while the head's
        // distance from (5, 5) changes. Absolute center position must contribute no bonus.
        for health in [10, 35, 100] {
            let (outer, outer_you) = translated_board(0, health);
            let (inner, inner_you) = translated_board(1, health);
            assert_eq!(
                evaluate_board(&outer, &outer_you),
                evaluate_board(&inner, &inner_you)
            );
            assert_eq!(
                evaluate_board_with_food(&outer, &outer_you, &outer.get_all_food_as_positions()),
                evaluate_board_with_food(&inner, &inner_you, &inner.get_all_food_as_positions())
            );
        }
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
    fn supplied_food_list_matches_the_board_scan() {
        // Passing the board's own food list must score identically to letting the evaluator scan
        // for it, across every snake on every fixture and after a simulated step.
        for fixture in [
            include_str!("../fixtures/turn33-food.json"),
            include_str!("../../battlesnake-game-types/fixtures/start_of_game.json"),
            include_str!("../../battlesnake-game-types/fixtures/late_stage.json"),
            include_str!("../../battlesnake-game-types/fixtures/tail_chase.json"),
            include_str!("../../battlesnake-game-types/fixtures/goes_for_food.json"),
        ] {
            let game: Game = serde_json::from_str(fixture).expect("valid fixture");
            let ids = build_snake_id_map(&game);
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).expect("valid board");
            let moves: Vec<_> = board
                .reasonable_move_masks()
                .iter()
                .map(|(id, mask)| (*id, mv_for(mask)))
                .collect();
            let after = board.simulate_single_action(&moves).1;

            for subject in [board, after] {
                let food = subject.get_all_food_as_positions();
                for index in 0..4 {
                    let you = SnakeId(index);
                    assert_eq!(
                        evaluate_board_with_food(&subject, &you, &food),
                        evaluate_board(&subject, &you),
                        "{fixture} {you:?} supplied food list"
                    );
                }
            }
        }
    }

    fn mv_for(mask: &u8) -> Move {
        // `reasonable_move_masks` never yields an empty mask, so this always names a real move.
        Move::from_index(mask.trailing_zeros() as usize)
    }

    #[test]
    fn production_food_term_rewards_near_food_at_high_health() {
        let (close_board, you) = board_with_food(80, Position::new(9, 9));
        let (far_board, _) = board_with_food(80, Position::new(0, 0));

        assert!(
            evaluate_board(&close_board, &you) > evaluate_board(&far_board, &you),
            "the production evaluator should reward being closer to food at high health"
        );
    }

    #[test]
    fn food_distance_weight_is_independent_of_health() {
        let distance_delta = |health| {
            let (close_board, you) = board_with_food(health, Position::new(9, 9));
            let (far_board, _) = board_with_food(health, Position::new(0, 0));
            i32::from(evaluate_board(&close_board, &you))
                - i32::from(evaluate_board(&far_board, &you))
        };
        let expected = distance_delta(100);
        assert!(expected > 0);
        for health in [10, 19, 20, 29, 30, 39, 40, 50, 80] {
            assert_eq!(distance_delta(health), expected, "health={health}");
        }
    }
}
