#[path = "../benches/support/mod.rs"]
mod support;
use battlesnake_game_types::{types::build_snake_id_map, wire_representation::Game};
fn main() {
    let path = "lib/benches/fixtures/arena.json";
    let mut rows: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for f in support::fixtures() {
        let row = rows
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["name"] == f.name)
            .unwrap();
        let game: Game = serde_json::from_value(row["game"].clone()).unwrap();
        let ids = build_snake_id_map(&game);
        let mut names = vec![String::new(); 4];
        for (name, id) in ids {
            names[id.0 as usize] = name;
        }
        row["action_snakes"] = serde_json::json!(names);
        let mut rng = support::rng();
        let actions = (0..64)
            .map(|_| lib::mcts::bench::sample_rollout_moves(&f.board, &f.food, &mut rng))
            .collect::<Vec<_>>();
        row["actions"] = serde_json::json!(
            actions
                .iter()
                .map(|a| a
                    .iter()
                    .map(|(id, mv)| [id.0 as usize, mv.as_index()])
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>()
        );
    }
    let lines = rows
        .as_array()
        .unwrap()
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join(",\n");
    std::fs::write(path, format!("[\n{lines}\n]\n")).unwrap();
}
