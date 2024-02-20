mod engine;

use axum::body::Body;
use axum::extract::State;
use axum::handler::Handler;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{debug_handler, Json, Router};
use battlesnake_game_types::compact_representation::dimensions::Square;
use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::compact_representation::wrapped::CellBoard;
use battlesnake_game_types::types::{
    build_snake_id_map, Move, SnakeIDGettableGame, SnakeIDMap, VictorDeterminableGame,
};
use battlesnake_game_types::wire_representation::Game;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::error::Error;
use std::sync::{Arc, Mutex};
use tracing::info;

type GameStates = Arc<Mutex<HashMap<String, SnakeIDMap>>>;

fn decode_state(
    mut text: String,
    game_states: GameStates,
) -> color_eyre::Result<CellBoard4Snakes11x11> {
    #[cfg(debug_assertions)]
    info!("JSON: {}", text);
    let decoded: Game = unsafe { simd_json::serde::from_str(&mut text) }?;
    let cellboard = decoded
        .as_cell_board(
            game_states
                .lock()
                .unwrap()
                .get(&decoded.game.id)
                .expect("No such game id found"),
        )
        .unwrap();
    Ok(cellboard)
}

#[debug_handler]
async fn get_move(State(game_states): State<GameStates>, body: String) -> Json<Value> {
    let cellboard = decode_state(body, game_states).unwrap();
    Json(json!({"move": engine::calc_move(cellboard).to_string()}))
}

async fn info() -> Json<Value> {
    Json(json!({
        "apiversion": "1",
        "author": "Nereuxofficial",
        "color": "#888888", // TODO: Choose color
        "head": "default", // TODO: Choose head
        "tail": "default", // TODO: Choose tail
    }))
}

// TODO: Delete the game state from the game_states map
async fn end(body: String) -> Response {
    let game_state: Game = serde_json::from_str(&body).unwrap();
    info!("Winner is {:?}", game_state.get_winner());
    Response::default()
}

async fn start(State(game_states): State<GameStates>, body: String) -> Response {
    let game_state: Game = serde_json::from_str(&body).unwrap();
    info!(
        "Game started with {} snakes",
        game_state.get_snake_ids().len()
    );
    let mut snake_id_map = build_snake_id_map(&game_state);
    game_states
        .lock()
        .unwrap()
        .insert(game_state.game.id.clone(), snake_id_map);
    Response::default()
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    tracing_subscriber::fmt::init();
    color_eyre::install()?;
    dotenvy::dotenv().ok();
    let _guard = sentry::init((
        std::env::var("SENTRY_DSN").unwrap(),
        sentry::ClientOptions {
            release: sentry::release_name!(),
            ..Default::default()
        },
    ));
    let gamestates: GameStates = GameStates::new(Mutex::new(HashMap::new()));
    info!("Hello Snakes!");
    let app = Router::new()
        .route("/", get(info))
        .route("/move", post(get_move))
        .route("/info", get(info))
        .route("/start", post(start))
        .route("/end", post(end))
        .with_state(gamestates);
    let listener = tokio::net::TcpListener::bind(format!(
        "0.0.0.0:{}",
        std::env::var("PORT").expect("Please set the PORT environment variable")
    ))
    .await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_state() {
        let game = r##"{"game":{"id":"203cd476-bd6f-4c20-8021-b222043f16e5","ruleset":{"name":"standard","version":"v1.2.3","settings":{"foodSpawnChance":15,"minimumFood":1,"hazardDamagePerTurn":0,"hazardMap":"","hazardMapAuthor":"","royale":{"shrinkEveryNTurns":0},"squad":{"allowBodyCollisions":false,"sharedElimination":false,"sharedHealth":false,"sharedLength":false}}},"map":"standard","timeout":500,"source":"custom"},"turn":0,"board":{"height":11,"width":11,"snakes":[{"id":"gs_PcT4fXmg3KPK4wgGgph6YYG6","name":"Hungry Bot","latency":"","health":100,"body":[{"x":9,"y":5},{"x":9,"y":5},{"x":9,"y":5}],"head":{"x":9,"y":5},"length":3,"shout":"","squad":"","customizations":{"color":"00cc00","head":"alligator","tail":"alligator"}},{"id":"gs_kQTcFtrXvdhXBdSy6TCKqck3","name":"Loopy Bot","latency":"","health":100,"body":[{"x":5,"y":9},{"x":5,"y":9},{"x":5,"y":9}],"head":{"x":5,"y":9},"length":3,"shout":"","squad":"","customizations":{"color":"#800080","head":"caffeine","tail":"iguana"}},{"id":"gs_vKjjkVGxJpQchxG6tYMwQHCV","name":"bene-snake-dev","latency":"","health":100,"body":[{"x":5,"y":1},{"x":5,"y":1},{"x":5,"y":1}],"head":{"x":5,"y":1},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}},{"id":"gs_RFCHRmwRBWrhfQSX7x83JPcW","name":"Loopy Bot","latency":"","health":100,"body":[{"x":1,"y":5},{"x":1,"y":5},{"x":1,"y":5}],"head":{"x":1,"y":5},"length":3,"shout":"","squad":"","customizations":{"color":"#800080","head":"caffeine","tail":"iguana"}}],"food":[{"x":10,"y":4},{"x":6,"y":10},{"x":6,"y":0},{"x":0,"y":6},{"x":5,"y":5}],"hazards":[]},"you":{"id":"gs_vKjjkVGxJpQchxG6tYMwQHCV","name":"bene-snake-dev","latency":"","health":100,"body":[{"x":5,"y":1},{"x":5,"y":1},{"x":5,"y":1}],"head":{"x":5,"y":1},"length":3,"shout":"","squad":"","customizations":{"color":"#888888","head":"default","tail":"default"}}}"##;
        let game = decode_state(game.to_string()).unwrap();
    }
}
