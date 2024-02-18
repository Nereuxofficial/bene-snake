mod engine;

use axum::body::Body;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{debug_handler, Json, Router};
use battlesnake_game_types::compact_representation::wrapped::CellBoard;
use battlesnake_game_types::compact_representation::{
    StandardCellBoard, StandardCellBoard4Snakes11x11,
};
use battlesnake_game_types::types::{
    Move, SnakeIDGettableGame, SnakeIDMap, VictorDeterminableGame,
};
use battlesnake_game_types::wire_representation::Game;
use serde_json::{json, Value};
use std::error::Error;
use tracing::info;

fn decode_state(mut text: String) -> color_eyre::Result<StandardCellBoard4Snakes11x11> {
    let decoded: Game = unsafe { simd_json::serde::from_str(&mut text) }?;
    let cellboard: StandardCellBoard4Snakes11x11 =
        decoded.as_cell_board(&SnakeIDMap::new()).unwrap();
    Ok(cellboard)
}

async fn get_move(body: String) -> Json<Value> {
    let cellboard = decode_state(body).unwrap();
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

async fn end(body: String) -> Response {
    let game_state: Game = serde_json::from_str(&body).unwrap();
    info!("Winner is {:?}", game_state.get_winner());
    Response::default()
}

async fn start(body: String) -> Response {
    let game_state: Game = serde_json::from_str(&body).unwrap();
    info!(
        "Game started with {} snakes",
        game_state.get_snake_ids().len()
    );
    Response::default()
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    tracing_subscriber::fmt::init();
    color_eyre::install()?;
    dotenvy::dotenv().ok();
    info!("Hello Snakes!");
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
