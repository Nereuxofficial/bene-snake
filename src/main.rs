#![feature(nonpoison_mutex)]
#![feature(sync_nonpoison)]

use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::{
    Move, SnakeIDGettableGame, SnakeIDMap, YouDeterminableGame, build_snake_id_map,
};
use battlesnake_game_types::wire_representation::Game;
use git_version::git_version;
use lib::mcts::{Node, mcts_search};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::nonpoison::Mutex;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tracing::{error, info};

pub static GAME_STATES: OnceLock<Mutex<BTreeMap<String, SnakeIDMap>>> = OnceLock::new();
static LAST_GAME_REQUEST: OnceLock<Mutex<Instant>> = OnceLock::new();
const DEPLOY_QUIET_PERIOD: Duration = Duration::from_secs(60);
// Leave room for response serialization and the public network path.
const RESPONSE_RESERVE: Duration = Duration::from_millis(75);
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub fn decode_state(text: String) -> color_eyre::Result<(CellBoard4Snakes11x11, i64)> {
    record_game_request();
    let game: Game = serde_json::from_str(&text)?;
    let mut binding = GAME_STATES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock();
    let snake_id_map = binding.entry(game.game.id.clone()).or_insert_with(|| {
        info!(
            "Game {} had no /start request; initializing from /move",
            game.game.id
        );
        build_snake_id_map(&game)
    });
    Ok((game.as_cell_board(snake_id_map).unwrap(), game.game.timeout))
}

fn search_budget(game_timeout_ms: i64, elapsed: Duration) -> Duration {
    Duration::from_millis(game_timeout_ms.max(0) as u64)
        .saturating_sub(RESPONSE_RESERVE)
        .saturating_sub(elapsed)
}

struct StopSearch(Arc<AtomicBool>);

impl Drop for StopSearch {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn record_game_request() {
    *LAST_GAME_REQUEST
        .get_or_init(|| Mutex::new(Instant::now()))
        .lock() = Instant::now();
}

async fn deploy_ready() -> axum::http::StatusCode {
    let active_games = GAME_STATES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .len();
    let quiet_for = LAST_GAME_REQUEST
        .get_or_init(|| Mutex::new(Instant::now()))
        .lock()
        .elapsed();
    if active_games == 0 && quiet_for >= DEPLOY_QUIET_PERIOD {
        axum::http::StatusCode::NO_CONTENT
    } else {
        axum::http::StatusCode::CONFLICT
    }
}

async fn get_move(body: String) -> Json<Value> {
    let start = std::time::Instant::now();
    info!("Got move request: {}", body);
    let (board, game_timeout_ms) = decode_state(body).unwrap();
    let you = *board.you_id();
    let root_node = Arc::new(Node::new_root(board));
    let root_node_clone = root_node.clone();
    let stop = StopSearch(Arc::new(AtomicBool::new(false)));
    let stop_for_search = Arc::clone(&stop.0);
    let task = tokio::task::spawn_blocking(move || {
        mcts_search(root_node_clone, &you, stop_for_search);
    });
    tokio::time::sleep(search_budget(game_timeout_ms, start.elapsed())).await;
    drop(stop);
    let mut failed = false;
    let chosen_move = root_node.best_move(you).unwrap_or_else(|| {
        failed = true;
        info!("Could not get move in game!");
        Move::Down
    });
    info!("Got move {chosen_move} in {:?}", start.elapsed());
    if failed
        && task.is_finished()
        && let Err(e) = task.await
    {
        error!("MCTS Search failed with: {e}");
    }
    Json(json!({"move": chosen_move}))
}

async fn info() -> Json<Value> {
    let rev = git_version!(fallback = env!("GIT_REVISION"));
    Json(json!({
        "apiversion": "1",
        "author": "Nereuxofficial",
        "color": "#FF5E5B",
        "head": "ferret",
        "tail": "curled",
        "rev": rev
    }))
}

async fn end(body: String) -> Response {
    record_game_request();
    let game_state: Game = serde_json::from_str(&body).unwrap();
    if game_state.you_are_winner() {
        info!("We won the game {}", game_state.game.id);
    } else {
        info!("We lost the game {}", game_state.game.id);
    }

    GAME_STATES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .remove(&game_state.game.id);

    Response::default()
}

async fn start(body: String) -> Response {
    record_game_request();
    let game_state: Game = serde_json::from_str(&body).unwrap();
    info!(
        "Game {} started with {} snakes",
        body,
        game_state.get_snake_ids().len()
    );
    GAME_STATES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .entry(game_state.game.id.clone())
        .or_insert_with(|| build_snake_id_map(&game_state));
    Response::default()
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    dotenvy::dotenv().ok();
    let mut sentry_options = sentry::ClientOptions::default();
    sentry_options.release = sentry::release_name!();
    let _guard = sentry::init((std::env::var("GLITCHTIP_KEY").unwrap(), sentry_options));

    tracing_subscriber::fmt().init();

    let addr = format!(
        "0.0.0.0:{}",
        std::env::var("PORT").expect("Please set the PORT environment variable")
    );
    info!("Starting battle-snake server on http://{addr}");
    let app = Router::new()
        .route("/", get(info))
        .route("/move", post(get_move))
        .route("/info", get(info))
        .route("/start", post(start))
        .route("/end", post(end))
        .route("/deploy-ready", get(deploy_ready));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_budget_reserves_time_for_the_response() {
        assert_eq!(
            search_budget(500, Duration::ZERO),
            Duration::from_millis(330)
        );
        assert_eq!(
            search_budget(500, Duration::from_millis(75)),
            Duration::from_millis(255)
        );
        assert_eq!(search_budget(40, Duration::ZERO), Duration::ZERO);
    }

    #[tokio::test]
    async fn move_response_uses_lowercase_move_names() {
        let body = include_str!("../lib/fixtures/turn33-food.json").to_string();
        let game: Game = serde_json::from_str(&body).expect("valid fixture");
        let snake_id_map = build_snake_id_map(&game);
        GAME_STATES
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .insert(game.game.id.clone(), snake_id_map);

        let Json(value) = get_move(body).await;
        let mv = value
            .get("move")
            .and_then(Value::as_str)
            .expect("response must contain a string move");
        assert!(
            ["up", "down", "left", "right"].contains(&mv),
            "expected one of the four lowercase moves, got {mv:?}"
        );
    }

    #[tokio::test]
    async fn move_without_start_initializes_game_state() {
        let mut game: Game = serde_json::from_str(include_str!("../lib/fixtures/turn33-food.json"))
            .expect("valid fixture");
        game.game.id = "missing-start-regression".to_string();
        let body = serde_json::to_string(&game).expect("serialize game");

        let Json(response) = get_move(body).await;
        assert!(
            ["up", "down", "left", "right"].contains(&response["move"].as_str().unwrap()),
            "expected a valid move, got {response:?}"
        );
        assert_eq!(
            GAME_STATES.get().unwrap().lock()[&game.game.id][&game.you.id].0,
            0
        );

        GAME_STATES.get().unwrap().lock().remove(&game.game.id);
    }

    #[tokio::test]
    async fn cancelled_request_stops_its_blocking_worker() {
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel();
        let request = tokio::spawn(async move {
            let stop = StopSearch(Arc::new(AtomicBool::new(false)));
            let worker_stop = Arc::clone(&stop.0);
            tokio::task::spawn_blocking(move || {
                while !worker_stop.load(Ordering::Relaxed) {
                    std::thread::yield_now();
                }
                let _ = stopped_tx.send(());
            });
            let _ = ready_tx.send(());
            std::future::pending::<()>().await;
        });

        ready_rx.await.unwrap();
        request.abort();
        let _ = request.await;
        tokio::time::timeout(Duration::from_secs(1), stopped_rx)
            .await
            .expect("worker did not stop after request cancellation")
            .unwrap();
    }
}
