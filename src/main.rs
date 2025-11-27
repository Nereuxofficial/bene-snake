use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use battlesnake_game_types::types::{build_snake_id_map, SnakeIDGettableGame};
use battlesnake_game_types::wire_representation::Game;
use lib::{calc_move, decode_state, GameStates};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::info;
// TODO: Implement MCTS

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

async fn get_move(State(game_states): State<GameStates>, body: String) -> Json<Value> {
    let start = std::time::Instant::now();
    info!("Got move request: {}", body);
    let cellboard = decode_state(body, game_states).unwrap();
    let chosen_move = calc_move(cellboard, 55, start).to_string();
    info!("Calculation took: {:?}", start.elapsed());
    Json(json!({"move": chosen_move}))
}

async fn info() -> Json<Value> {
    Json(json!({
        "apiversion": "1",
        "author": "Nereuxofficial",
        "color": "#FF5E5B",
        "head": "ferret",
        "tail": "curled",
    }))
}

async fn end(State(game_states): State<GameStates>, body: String) -> Response {
    let game_state: Game = serde_json::from_str(&body).unwrap();
    if game_state.you_are_winner() {
        info!("We won the game");
    } else {
        info!("We lost the game");
    }

    game_states.lock().unwrap().remove(&game_state.game.id);
    Response::default()
}

async fn start(State(game_states): State<GameStates>, body: String) -> Response {
    let game_state: Game = serde_json::from_str(&body).unwrap();
    info!(
        "Game {} started with {} snakes",
        body,
        game_state.get_snake_ids().len()
    );
    let snake_id_map = build_snake_id_map(&game_state);
    game_states
        .lock()
        .unwrap()
        .insert(game_state.game.id.clone(), snake_id_map);
    Response::default()
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    dotenvy::dotenv().ok();
    let _guard = sentry::init((
        std::env::var("SENTRY_DSN").unwrap(),
        sentry::ClientOptions {
            release: sentry::release_name!(),
            traces_sample_rate: 0.0,
            ..Default::default()
        },
    ));

    tracing_subscriber::fmt().init();

    let gamestates: GameStates = GameStates::new(Mutex::new(HashMap::new()));
    info!("Starting battle-snake server...");
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
