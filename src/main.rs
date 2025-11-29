use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::{
    build_snake_id_map, Move, SnakeIDGettableGame, SnakeIDMap, YouDeterminableGame,
};
use battlesnake_game_types::wire_representation::Game;
use lib::mcts::{mcts_search, Node};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tracing::info;

pub static GAME_STATES: OnceLock<Mutex<BTreeMap<String, SnakeIDMap>>> = OnceLock::new();
pub const PING: u64 = 60;
pub const TIME_TO_MOVE: u64 = 500 - 2 * PING;
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub fn decode_state(text: String) -> color_eyre::Result<CellBoard4Snakes11x11> {
    let game: Game = serde_json::from_str(&text)?;
    let binding = GAME_STATES.get().unwrap().lock().unwrap();
    let snake_id_map = binding.get(&game.game.id).unwrap();
    Ok(game.as_cell_board(snake_id_map).unwrap())
}
async fn get_move(body: String) -> Json<Value> {
    let start = std::time::Instant::now();
    info!("Got move request: {}", body);
    let board = decode_state(body).unwrap();
    let you = board.you_id().clone();
    let root_node = Arc::new(Node::new_root(board.clone()));
    let root_node_clone = root_node.clone();
    let stop_bool = Arc::new(AtomicBool::new(false));
    let stop_bool_ref = stop_bool.clone();
    let task = tokio::task::spawn_blocking(move || {
        mcts_search(root_node_clone, &you, stop_bool_ref);
    });
    tokio::time::sleep(Duration::from_millis(TIME_TO_MOVE)).await;
    stop_bool.store(true, Ordering::Relaxed);
    let chosen_move = root_node
        .best_child(0.0)
        .map(|c| c.0.own_move())
        .unwrap_or_else(|| {
            info!("Could not get move in game!");
            Move::Down
        });
    info!(
        "Got move {chosen_move} in {:?} with depth {}",
        start.elapsed(),
        root_node.get_depth()
    );
    let lowercase_move = chosen_move.to_string().to_lowercase();
    Json(json!({"move": lowercase_move}))
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

async fn end(body: String) -> Response {
    let game_state: Game = serde_json::from_str(&body).unwrap();
    if game_state.you_are_winner() {
        info!("We won the game {}", game_state.game.id);
    } else {
        info!("We lost the game {}", game_state.game.id);
    }

    Response::default()
}

async fn start(body: String) -> Response {
    let game_state: Game = serde_json::from_str(&body).unwrap();
    info!(
        "Game {} started with {} snakes",
        body,
        game_state.get_snake_ids().len()
    );
    let snake_id_map = build_snake_id_map(&game_state);
    GAME_STATES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
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

    info!("Starting battle-snake server...");
    let app = Router::new()
        .route("/", get(info))
        .route("/move", post(get_move))
        .route("/info", get(info))
        .route("/start", post(start))
        .route("/end", post(end));
    let listener = tokio::net::TcpListener::bind(format!(
        "0.0.0.0:{}",
        std::env::var("PORT").expect("Please set the PORT environment variable")
    ))
    .await?;
    axum::serve(listener, app).await?;
    Ok(())
}
