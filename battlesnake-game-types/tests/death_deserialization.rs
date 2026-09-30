use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11, types::build_snake_id_map,
    wire_representation::Game,
};
use serde_json::json;

#[test]
fn game_excludes_eliminated_snakes_before_board_conversion() {
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/start_of_game.json")).unwrap();
    let living = payload["you"].clone();
    let mut living_with_null_death = living.clone();
    living_with_null_death["id"] = json!("living-null");
    living_with_null_death["Death"] = serde_json::Value::Null;
    let mut dead = living.clone();
    dead["id"] = json!("dead");
    dead["head"] = json!({"x": 1, "y": 11});
    dead["body"] = json!([{"x": 1, "y": 11}]);
    dead["Death"] = json!({"Cause": "wall-collision", "Turn": 1});
    let mut lowercase_dead = dead.clone();
    lowercase_dead["id"] = json!("lowercase-dead");
    lowercase_dead.as_object_mut().unwrap().remove("Death");
    lowercase_dead["death"] = json!({"Cause": "wall-collision", "Turn": 1});
    payload["board"]["snakes"] = json!([living, dead, living_with_null_death, lowercase_dead]);

    let mut game: Game = serde_json::from_value(payload).unwrap();
    assert_eq!(game.board.snakes.len(), 2);
    assert_eq!(game.board.snakes[0].id, game.you.id);
    assert_eq!(game.board.snakes[1].id, "living-null");
    assert!(game.board.snakes.iter().all(|snake| snake.health > 0));
    // The retained test snakes share a body; remove the synthetic second living
    // snake before checking that the off-board eliminated snake never reaches conversion.
    game.board.snakes.truncate(1);
    let ids = build_snake_id_map(&game);
    let _: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
    let roundtrip: Game = serde_json::from_value(serde_json::to_value(&game).unwrap()).unwrap();
    assert_eq!(game, roundtrip);
}
