#![feature(nonpoison_mutex)]
#![feature(sync_nonpoison)]

mod game_state;

use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::{Move, SnakeIDGettableGame, YouDeterminableGame};
use battlesnake_game_types::wire_representation::Game;
use game_state::{GameState, ResponseMoves, response_moves};
use git_version::git_version;
use lib::mcts::{Node, SearchTreeCache, mcts_search_with_publish};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::nonpoison::Mutex;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tracing::{error, info};

static GAME_STATES: OnceLock<Mutex<BTreeMap<String, GameState>>> = OnceLock::new();
static TREE_CACHES: OnceLock<Mutex<BTreeMap<String, CachedSearch>>> = OnceLock::new();
static LAST_GAME_REQUEST: OnceLock<Mutex<Instant>> = OnceLock::new();
const DEPLOY_QUIET_PERIOD: Duration = Duration::from_secs(60);
const TREE_CACHE_TTL: Duration = Duration::from_secs(90);
const MAX_CACHED_GAMES: usize = 16;
const GAME_STATE_TTL: Duration = Duration::from_secs(300);
// Leave room for response serialization and the public network path.
const RESPONSE_RESERVE: Duration = Duration::from_millis(70);
// Spend at most 20 ms of that reserve waiting for a stopped worker's published result.
const SEARCH_STOP_GRACE: Duration = Duration::from_millis(20);
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

struct DecodedState {
    board: Option<CellBoard4Snakes11x11>,
    response_moves: ResponseMoves,
    timeout_ms: i64,
    game_id: String,
    turn: i32,
}

struct CachedSearch {
    turn: i32,
    candidates: SearchTreeCache,
    touched: Instant,
}

fn tree_caches() -> &'static Mutex<BTreeMap<String, CachedSearch>> {
    TREE_CACHES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn take_search_root(
    game_id: &str,
    turn: i32,
    board: CellBoard4Snakes11x11,
    you: battlesnake_game_types::types::SnakeId,
) -> (Arc<Node>, bool) {
    let cached = tree_caches().lock().remove(game_id);
    if let Some(cached) = cached
        && cached.turn.checked_add(1) == Some(turn)
        && cached.touched.elapsed() <= TREE_CACHE_TTL
        && let Some(root) = cached.candidates.match_observed(&board, you)
    {
        return (root, true);
    }
    (Arc::new(Node::new_root(board)), false)
}

fn store_search_candidates(game_id: String, turn: i32, candidates: SearchTreeCache) {
    if candidates.candidate_count() == 0 {
        return;
    }
    // Keep this guard while inserting: /end removes the game before clearing its tree.
    let games = GAME_STATES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock();
    if !games.contains_key(&game_id) {
        return;
    }
    let mut caches = tree_caches().lock();
    caches.retain(|_, cached| cached.touched.elapsed() <= TREE_CACHE_TTL);
    if caches
        .get(&game_id)
        .is_some_and(|cached| cached.turn > turn)
    {
        return;
    }
    if !caches.contains_key(&game_id)
        && caches.len() >= MAX_CACHED_GAMES
        && let Some(oldest) = caches
            .iter()
            .min_by_key(|(_, cached)| cached.touched)
            .map(|(id, _)| id.clone())
    {
        caches.remove(&oldest);
    }
    caches.insert(
        game_id,
        CachedSearch {
            turn,
            candidates,
            touched: Instant::now(),
        },
    );
}

fn decode_state(text: String) -> color_eyre::Result<DecodedState> {
    record_game_request();
    let mut game: Game = serde_json::from_str(&text)?;
    let mut games = GAME_STATES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock();
    games.retain(|_, state| state.touched.elapsed() <= GAME_STATE_TTL);
    let state = games.entry(game.game.id.clone()).or_insert_with(|| {
        info!(game_id = %game.game.id, turn = game.turn, "Initializing game state from /move");
        GameState::new(&game)
    });
    let original_snakes = game.board.snakes.len();
    let normalized = state.normalize(&mut game);
    let ids = state.ids.clone();
    drop(games);
    let response_moves = response_moves(&game);
    let board = match normalized {
        Err(e) => {
            error!(game_id = %game.game.id, turn = game.turn, error = %e, "Invalid normalized board; using fallback");
            None
        }
        Ok(()) => {
            info!(game_id = %game.game.id, turn = game.turn,
                removed_snakes = original_snakes - game.board.snakes.len(), "Normalized move request");
            // This server searches a fixed 11x11 board; reject unsupported shapes
            // before converting indices. Malformed requests must not panic the handler.
            if game.board.width != 11 || game.board.height != 11 {
                error!(game_id = %game.game.id, turn = game.turn, "Unsupported search dimensions; using fallback");
                None
            } else {
                match catch_unwind(AssertUnwindSafe(|| game.as_cell_board(&ids))) {
                    Ok(Ok(board)) => Some(board),
                    _ => {
                        error!(game_id = %game.game.id, turn = game.turn, "Compact conversion failed; using fallback");
                        None
                    }
                }
            }
        }
    };
    Ok(DecodedState {
        board,
        response_moves,
        timeout_ms: game.game.timeout,
        game_id: game.game.id,
        turn: game.turn,
    })
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
    let active_games = {
        let mut games = GAME_STATES
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock();
        games.retain(|_, state| state.touched.elapsed() <= GAME_STATE_TTL);
        games.len()
    };
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
    let decoded = match decode_state(body) {
        Ok(decoded) => decoded,
        Err(e) => {
            error!(error = %e, "Unparseable move request; no board available for fallback");
            return Json(json!({"move": Move::Up}));
        }
    };
    let fallback = decoded.response_moves.fallback;
    let Some(board) = decoded.board else {
        return Json(json!({"move": fallback}));
    };
    let you = *board.you_id();
    let root = catch_unwind(AssertUnwindSafe(|| {
        take_search_root(&decoded.game_id, decoded.turn, board, you)
    }));
    let Ok((root_node, reused)) = root else {
        error!(game_id = %decoded.game_id, turn = decoded.turn, "Search initialization failed; using fallback");
        return Json(json!({"move": fallback}));
    };
    let carried_visits = root_node.visits();
    let stop = StopSearch(Arc::new(AtomicBool::new(false)));
    let stop_for_search = Arc::clone(&stop.0);
    let game_id = decoded.game_id.clone();
    let turn = decoded.turn;
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let worker_moves = ResponseMoves {
        fallback,
        acceptable: decoded.response_moves.acceptable,
    };
    tokio::task::spawn_blocking(move || {
        let _span = tracing::info_span!("search", game_id = %game_id, turn).entered();
        mcts_search_with_publish(root_node.clone(), &you, stop_for_search, || {
            // Only this worker reads its tree, after successful search completion.
            let result = validated_search_result(true, &worker_moves, || {
                let chosen = root_node.best_move(you)?;
                Some((chosen, SearchTreeCache::after_move(&root_node, you, chosen)))
            });
            // The handler need not wait for telemetry or destruction of the unused tree.
            let _ = result_tx.send(result);
        });
    });
    let budget = search_budget(decoded.timeout_ms, start.elapsed());
    // Short deadlines retain the same delivery cushion: do not wait past timeout - 165 ms.
    let delivery_reserve = RESPONSE_RESERVE - SEARCH_STOP_GRACE;
    let available = Duration::from_millis(decoded.timeout_ms.max(0) as u64)
        .saturating_sub(start.elapsed())
        .saturating_sub(delivery_reserve);
    let grace = available.saturating_sub(budget).min(SEARCH_STOP_GRACE);
    let outcome = wait_for_search(result_rx, stop, budget, grace).await;
    let result_published = outcome.is_some();
    let mut chosen_move = fallback;
    let mut retained_candidates = 0;
    let mut used_fallback = true;
    match outcome {
        Some(Some((chosen, candidates))) => {
            chosen_move = chosen;
            used_fallback = false;
            retained_candidates = candidates.candidate_count();
            store_search_candidates(decoded.game_id.clone(), decoded.turn, candidates);
        }
        Some(None) => {
            error!(game_id = %decoded.game_id, turn = decoded.turn, "Invalid search result; using fallback");
        }
        None => {
            error!(game_id = %decoded.game_id, turn = decoded.turn, "Search worker failed or did not publish; using fallback");
        }
    }
    info!(game_id = %decoded.game_id, turn = decoded.turn,
        chosen_move = %chosen_move, elapsed = ?start.elapsed(), reused, carried_visits,
        retained_candidates, result_published, used_fallback, "MCTS move completed");
    Json(json!({"move": chosen_move}))
}

fn validated_search_result(
    worker_finished: bool,
    moves: &ResponseMoves,
    publish: impl FnOnce() -> Option<(Move, SearchTreeCache)>,
) -> Option<(Move, SearchTreeCache)> {
    if !worker_finished {
        return None;
    }
    catch_unwind(AssertUnwindSafe(publish))
        .ok()
        .flatten()
        .filter(|(chosen, _)| moves.acceptable[chosen.as_index()])
}

async fn wait_for_search<T>(
    mut result: tokio::sync::oneshot::Receiver<T>,
    stop: StopSearch,
    budget: Duration,
    grace: Duration,
) -> Option<T> {
    let search_deadline = tokio::time::Instant::now() + budget;
    let publication_deadline = search_deadline + grace;
    tokio::select! {
        result = &mut result => { drop(stop); result.ok() }
        _ = tokio::time::sleep_until(search_deadline) => {
            drop(stop);
            tokio::time::timeout_at(publication_deadline, result).await.ok().and_then(Result::ok)
        }
    }
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
    tree_caches().lock().remove(&game_state.game.id);

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
    let mut games = GAME_STATES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock();
    let new_game = !games.contains_key(&game_state.game.id);
    games
        .entry(game_state.game.id.clone())
        .or_insert_with(|| GameState::new(&game_state));
    // /start may arrive after /move; preserve its search tree in that case.
    if new_game {
        tree_caches().lock().remove(&game_state.game.id);
    }
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
            Duration::from_millis(315)
        );
        assert_eq!(
            search_budget(500, Duration::from_millis(75)),
            Duration::from_millis(240)
        );
        assert_eq!(search_budget(40, Duration::ZERO), Duration::ZERO);
    }

    #[tokio::test]
    async fn move_response_uses_lowercase_move_names() {
        let body = include_str!("../lib/fixtures/turn33-food.json").to_string();
        let game: Game = serde_json::from_str(&body).expect("valid fixture");
        GAME_STATES
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .insert(game.game.id.clone(), GameState::new(&game));

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
            GAME_STATES.get().unwrap().lock()[&game.game.id].ids[&game.you.id].0,
            0
        );

        GAME_STATES.get().unwrap().lock().remove(&game.game.id);
    }

    #[tokio::test]
    async fn late_start_preserves_search_cache_and_end_clears_it() {
        let mut game: Game = serde_json::from_str(include_str!("../lib/fixtures/turn33-food.json"))
            .expect("valid fixture");
        game.game.id = "late-start-tree-cache-regression".to_string();
        let ids = battlesnake_game_types::types::build_snake_id_map(&game);
        let board = game.as_cell_board(&ids).expect("compact board");
        let you = *board.you_id();
        GAME_STATES
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .insert(game.game.id.clone(), GameState::new(&game));
        tree_caches().lock().insert(
            game.game.id.clone(),
            CachedSearch {
                turn: game.turn,
                candidates: SearchTreeCache::after_move(
                    &Arc::new(Node::new_root(board)),
                    you,
                    Move::Up,
                ),
                touched: Instant::now(),
            },
        );

        let body = serde_json::to_string(&game).unwrap();
        start(body.clone()).await;
        assert!(tree_caches().lock().contains_key(&game.game.id));
        end(body).await;
        assert!(!tree_caches().lock().contains_key(&game.game.id));
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
    #[tokio::test]
    async fn panicked_worker_returns_fallback_without_reading_tree() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::task::spawn_blocking(move || {
            let _tx = tx;
            panic!("injected worker failure");
        });
        let finished = wait_for_search(
            rx,
            StopSearch(Arc::new(AtomicBool::new(false))),
            Duration::from_secs(1),
            SEARCH_STOP_GRACE,
        )
        .await
        .is_some();
        let moves = ResponseMoves {
            fallback: Move::Left,
            acceptable: [true; 4],
        };
        let result = validated_search_result(finished, &moves, || {
            panic!("failed tree must never be inspected")
        });
        assert_eq!(
            result.map(|(mv, _)| mv).unwrap_or(moves.fallback),
            Move::Left
        );
    }

    #[tokio::test]
    async fn worker_that_does_not_stop_is_not_read_or_cached() {
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::task::spawn_blocking(move || {
            let _tx = tx;
            let _ = release_rx.recv();
        });
        let flag = Arc::new(AtomicBool::new(false));
        let finished = wait_for_search(
            rx,
            StopSearch(flag.clone()),
            Duration::ZERO,
            SEARCH_STOP_GRACE,
        )
        .await
        .is_some();
        assert!(!finished);
        assert!(flag.load(Ordering::Relaxed));
        let moves = ResponseMoves {
            fallback: Move::Right,
            acceptable: [true; 4],
        };
        assert!(
            validated_search_result(finished, &moves, || panic!(
                "running tree must never be inspected"
            ))
            .is_none()
        );
        release_tx.send(()).unwrap();
    }

    #[tokio::test]
    async fn published_result_does_not_wait_for_worker_cleanup() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = tokio::task::spawn_blocking(move || {
            tx.send(Move::Left).unwrap();
            // Simulate slow telemetry or tree destruction after publication.
            release_rx.recv().unwrap();
        });
        let result = wait_for_search(
            rx,
            StopSearch(Arc::new(AtomicBool::new(false))),
            Duration::from_secs(1),
            SEARCH_STOP_GRACE,
        )
        .await;
        assert_eq!(result, Some(Move::Left));
        assert!(!worker.is_finished());
        release_tx.send(()).unwrap();
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn result_published_after_stop_is_received_within_grace() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let flag = Arc::new(AtomicBool::new(false));
        let worker_flag = flag.clone();
        let worker = tokio::task::spawn_blocking(move || {
            while !worker_flag.load(Ordering::Relaxed) {
                std::thread::yield_now();
            }
            tx.send(Move::Right).unwrap();
        });
        assert_eq!(
            wait_for_search(rx, StopSearch(flag), Duration::ZERO, Duration::from_secs(1)).await,
            Some(Move::Right)
        );
        worker.await.unwrap();
    }

    #[test]
    fn panicked_result_publication_uses_fallback() {
        let moves = ResponseMoves {
            fallback: Move::Down,
            acceptable: [true; 4],
        };
        let result = validated_search_result(true, &moves, || panic!("poisoned tree publication"));
        assert_eq!(
            result.map(|(mv, _)| mv).unwrap_or(moves.fallback),
            Move::Down
        );
    }

    #[tokio::test]
    async fn unsupported_conversion_returns_wire_fallback() {
        let mut game: Game =
            serde_json::from_str(include_str!("../lib/fixtures/turn33-food.json")).unwrap();
        game.game.id = "unsupported-conversion-regression".into();
        game.board.width = 12;
        let fallback = response_moves(&game).fallback;
        let Json(response) = get_move(serde_json::to_string(&game).unwrap()).await;
        assert_eq!(response["move"], serde_json::to_value(fallback).unwrap());
        GAME_STATES.get().unwrap().lock().remove(&game.game.id);
    }
}
