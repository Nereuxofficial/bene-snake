#![feature(nonpoison_mutex)]
#![feature(sync_nonpoison)]

mod game_state;
mod ponder;
mod search;

use axum::body::{Body, Bytes, HttpBody};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::{Move, SnakeIDGettableGame, YouDeterminableGame};
use battlesnake_game_types::wire_representation::Game;
use game_state::{GameState, ResponseMoves, response_moves};
use git_version::git_version;
use lib::mcts::{Node, SearchTreeCache};
use ponder::{PendingPonder, TREE_CACHE_TTL, TreeCaches};
use search::{PreparedSearch, SearchResult};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::nonpoison::Mutex;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tracing::{error, info};

static GAME_STATES: OnceLock<Mutex<BTreeMap<String, GameState>>> = OnceLock::new();
static TREE_CACHES: OnceLock<TreeCaches> = OnceLock::new();
static LAST_GAME_REQUEST: OnceLock<Mutex<Instant>> = OnceLock::new();
const DEPLOY_QUIET_PERIOD: Duration = Duration::from_secs(60);
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
    search_generation: Arc<()>,
}

fn tree_caches() -> &'static TreeCaches {
    TREE_CACHES.get_or_init(TreeCaches::new)
}

fn take_search_root(
    decoded: &DecodedState,
    board: CellBoard4Snakes11x11,
    you: battlesnake_game_types::types::SnakeId,
) -> (Arc<Node>, bool, u64) {
    let games = GAME_STATES.get().unwrap().lock();
    let current = games
        .get(&decoded.game_id)
        .is_some_and(|state| Arc::ptr_eq(&state.search_generation, &decoded.search_generation));
    let cached = current
        .then(|| tree_caches().take(&decoded.game_id))
        .flatten();
    drop(games);
    let pondered_iterations = cached.as_ref().map_or(0, |cache| cache.pondered_iterations);
    if let Some(cached) = cached
        && cached.turn.checked_add(1) == Some(decoded.turn)
        && cached.touched.elapsed() <= TREE_CACHE_TTL
        && let Some(root) = cached.candidates.match_observed(&board, you)
    {
        return (root, true, cached.pondered_iterations);
    }
    (Arc::new(Node::new_root(board)), false, pondered_iterations)
}

fn store_search_candidates(
    decoded: &DecodedState,
    candidates: SearchTreeCache,
) -> Option<PendingPonder> {
    // Keep this guard while inserting: /end removes the game before clearing its tree.
    let games = GAME_STATES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock();
    if !games
        .get(&decoded.game_id)
        .is_some_and(|state| Arc::ptr_eq(&state.search_generation, &decoded.search_generation))
    {
        return None;
    }
    tree_caches().insert(decoded.game_id.clone(), decoded.turn, candidates)
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
    let search_generation = Arc::new(());
    if game.turn >= state.search_turn {
        state.search_turn = game.turn;
        state.search_generation = Arc::clone(&search_generation);
    }
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
        search_generation,
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

async fn get_move(body: String) -> Response {
    get_move_with_search(body, |search, stop, result| {
        search.run(stop, |outcome| {
            // Send before telemetry and destruction of the unused tree.
            let _ = result.send(outcome);
        });
    })
    .await
}

async fn get_move_with_search(
    body: String,
    run_search: impl FnOnce(PreparedSearch, Arc<AtomicBool>, tokio::sync::oneshot::Sender<SearchResult>)
    + Send
    + 'static,
) -> Response {
    let start = std::time::Instant::now();
    // Decoding only normalises the request and records it; the tree is untouched, so it
    // can happen before admission without racing the previous turn's search.
    let decoded = match decode_state(body) {
        Ok(decoded) => decoded,
        Err(e) => {
            error!(error = %e, "Unparseable move request; no board available for fallback");
            return Json(json!({"move": Move::Up})).into_response();
        }
    };
    let _admitted = tree_caches().admit(&decoded.game_id).await;
    tree_caches().wait_idle().await;
    info!("Got move request for turn {}", decoded.turn);
    let fallback = decoded.response_moves.fallback;
    let Some(board) = decoded.board else {
        let games = GAME_STATES.get().unwrap().lock();
        if games
            .get(&decoded.game_id)
            .is_some_and(|state| Arc::ptr_eq(&state.search_generation, &decoded.search_generation))
        {
            tree_caches().remove(&decoded.game_id);
        }
        return Json(json!({"move": fallback})).into_response();
    };
    let you = *board.you_id();
    let root = catch_unwind(AssertUnwindSafe(|| take_search_root(&decoded, board, you)));
    let Ok((root_node, reused, pondered_iterations)) = root else {
        error!(game_id = %decoded.game_id, turn = decoded.turn, "Search initialization failed; using fallback");
        return Json(json!({"move": fallback})).into_response();
    };
    let carried_visits = root_node.visits();
    let search = PreparedSearch::new(root_node, you, &decoded.response_moves);
    let fallback = search.fallback();
    let stop = StopSearch(Arc::new(AtomicBool::new(false)));
    let stop_for_search = Arc::clone(&stop.0);
    let game_id = decoded.game_id.clone();
    let turn = decoded.turn;
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    // The admitted handler already holds the per-game lock; this pause only has to outlive
    // the handler future, which may end while the worker is still stopping.
    let worker_foreground = tree_caches().pause();
    tokio::task::spawn_blocking(move || {
        // A cancelled handler may return while its foreground workers are stopping.
        let _foreground = worker_foreground;
        let _span = tracing::info_span!("search", game_id = %game_id, turn).entered();
        run_search(search, stop_for_search, result_tx);
    });
    let budget = search_budget(decoded.timeout_ms, start.elapsed());
    // Short deadlines retain the same delivery cushion.
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
    let mut pending_ponder = None;
    match outcome {
        Some(Some((chosen, candidates))) => {
            chosen_move = chosen;
            used_fallback = false;
            retained_candidates = candidates.candidate_count();
            pending_ponder = store_search_candidates(&decoded, candidates);
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
        retained_candidates, pondered_iterations, result_published, used_fallback, "MCTS move completed");
    let response = Json(json!({"move": chosen_move})).into_response();
    if let Some(pending) = pending_ponder {
        let (parts, body) = response.into_parts();
        Response::from_parts(
            parts,
            Body::new(PonderResponse {
                body,
                pending: Some(pending),
                complete: false,
            }),
        )
    } else {
        response
    }
}

/// Start downtime work after the HTTP transport consumes the response body.
/// Dropping an unsent response must not start pondering for a cancelled request.
struct PonderResponse {
    body: Body,
    pending: Option<PendingPonder>,
    complete: bool,
}

impl HttpBody for PonderResponse {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, axum::Error>>> {
        let result = Pin::new(&mut self.body).poll_frame(cx);
        if matches!(result, Poll::Ready(Some(Err(_)))) {
            self.pending = None;
        } else if result.is_ready() && self.body.is_end_stream() {
            self.complete = true;
        }
        result
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }
    fn size_hint(&self) -> http_body::SizeHint {
        self.body.size_hint()
    }
}

impl Drop for PonderResponse {
    fn drop(&mut self) {
        if self.complete
            && let Some(pending) = self.pending.take()
        {
            tree_caches().response_sent(pending);
        }
    }
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
    let _foreground = tree_caches().pause();
    tree_caches().wait_idle().await;
    let game_state: Game = serde_json::from_str(&body).unwrap();
    if game_state.you_are_winner() {
        info!("We won the game {}", game_state.game.id);
    } else {
        info!("We lost the game {}", game_state.game.id);
        // TODO: Download for post-mortem analysis
    }

    GAME_STATES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .remove(&game_state.game.id);
    tree_caches().remove(&game_state.game.id);

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
        tree_caches().remove(&game_state.game.id);
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
    use crate::search::validated_search_result;

    #[tokio::test]
    async fn turn141_worker_failures_return_the_guarded_escape() {
        // Missing, invalid, and late publication must all use the move prepared
        // while the root was idle, without reading the worker's tree afterward.
        for failure in 0..3 {
            let mut game: Game =
                serde_json::from_str(include_str!("fixtures/68adaa6a-turn141.json")).unwrap();
            game.game.id = format!("guarded-turn141-failure-{failure}");
            let wire_moves = response_moves(&game);
            assert_eq!(wire_moves.fallback, Move::Left);
            assert!(wire_moves.acceptable[Move::Right.as_index()]);
            let body = serde_json::to_string(&game).unwrap();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let response = get_move_with_search(body.clone(), move |search, _, result| {
                assert_eq!(search.fallback(), Move::Right);
                match failure {
                    0 => drop(result), // Worker exited without publishing.
                    1 => {
                        let _ = result.send(None);
                    } // Invalid publication.
                    _ => {
                        // Keep both the tree and sender alive beyond the handler's
                        // deadline. The handler must return without either.
                        let _ = release_rx.recv();
                        drop(result);
                    }
                }
            })
            .await;
            // Release the fake worker before assertions so a failure cannot leave
            // the Tokio runtime waiting forever for a blocked spawn_blocking job.
            let _ = release_tx.send(());
            assert_eq!(
                response.headers()[axum::http::header::CONTENT_TYPE],
                "application/json"
            );
            let bytes = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["move"], "right", "worker failure mode {failure}");
            assert!(tree_caches().take(&game.game.id).is_none());
            end(body).await;
        }
    }

    async fn move_json(body: String) -> Value {
        let response = get_move(body).await;
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_TYPE],
            "application/json"
        );
        let bytes = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn searched_candidates(decoded: &DecodedState) -> SearchTreeCache {
        let board = decoded.board.unwrap();
        let you = *board.you_id();
        let root = Arc::new(Node::new_root(board));
        for _ in 0..300 {
            lib::mcts::search_once(&root, &you, &mut lib::mcts::SearchDepthStats::default());
        }
        let cache = SearchTreeCache::after_move(&root, you, root.best_move(you).unwrap());
        assert!(cache.candidate_count() > 0);
        cache
    }

    #[tokio::test]
    async fn move_downtime_adds_visits_that_the_next_request_reuses() {
        use battlesnake_game_types::types::{
            HeadGettableGame, HealthGettableGame, ReasonableMovesGame, SimulableGame,
        };
        let mut game: Game =
            serde_json::from_str(include_str!("../lib/fixtures/turn33-food.json")).unwrap();
        game.game.id = "downtime-reuse-integration".into();
        // The production timeout, not a tighter one. This test needs the search to publish
        // a tree before it can check anything about pondering, and a short budget starves
        // the workers when the whole suite runs in parallel on one shared search pool, so
        // the request falls back and never populates a cache. The search stops on its own
        // deadline, so a generous budget does not slow the suite down.
        game.game.timeout = 500;
        game.board.food.clear();
        let response = get_move(serde_json::to_string(&game).unwrap()).await;
        let ids = GAME_STATES.get().unwrap().lock()[&game.game.id].ids.clone();
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let you = *board.you_id();
        let (cache, iterations, ready) = tree_caches()
            .snapshot(&game.game.id)
            .expect("move retains searched replies");
        assert_eq!(iterations, 0);
        assert!(!ready);
        let replies: Vec<_> = board
            .simulate_with_moves(&board.reasonable_moves_for_each_snake())
            .filter_map(|(_, observed)| {
                cache
                    .match_observed(&observed, you)
                    .map(|root| (observed, root.visits(), root))
            })
            .collect();
        assert!(!replies.is_empty());
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while tree_caches().snapshot(&game.game.id).unwrap().1 == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("downtime must explore a next-turn tree");

        let _foreground = tree_caches().pause();
        tree_caches().wait_idle().await;
        let (observed, before, expected_root) = replies
            .into_iter()
            .find(|(_, visits, root)| root.visits() > *visits)
            .expect("pondering adds reusable visits");
        game.turn += 1;
        game.board
            .snakes
            .retain(|snake| observed.get_health(&ids[&snake.id]) > 0);
        for snake in &mut game.board.snakes {
            let id = ids[&snake.id];
            snake.head = observed.get_head_as_position(&id);
            snake.health = observed.get_health_i64(&id) as i32;
            snake.body.pop_back();
            snake.body.push_front(snake.head);
        }
        game.you = game
            .board
            .snakes
            .iter()
            .find(|snake| snake.id == game.you.id)
            .unwrap()
            .clone();
        let next_body = serde_json::to_string(&game).unwrap();
        let next = decode_state(next_body.clone()).unwrap();
        assert_eq!(next.board.unwrap(), observed);
        let (reused_root, reused, pondered_iterations) = take_search_root(&next, observed, you);
        assert!(reused);
        assert!(pondered_iterations > 0);
        assert!(Arc::ptr_eq(&reused_root, &expected_root));
        assert!(reused_root.visits() > before);
        end(next_body).await;
    }

    #[tokio::test]
    async fn only_consumed_move_responses_activate_pondering() {
        let mut game: Game =
            serde_json::from_str(include_str!("../lib/fixtures/turn33-food.json")).unwrap();
        game.game.id = "response-delivery-pondering".into();
        game.board.food.clear();
        let _foreground = tree_caches().pause();
        tree_caches().wait_idle().await;
        let decoded = decode_state(serde_json::to_string(&game).unwrap()).unwrap();
        let pending = store_search_candidates(&decoded, searched_candidates(&decoded)).unwrap();
        drop(PonderResponse {
            body: Body::from("unsent"),
            pending: Some(pending),
            complete: false,
        });
        assert!(!tree_caches().snapshot(&game.game.id).unwrap().2);

        let pending = store_search_candidates(&decoded, searched_candidates(&decoded)).unwrap();
        let body = Body::new(PonderResponse {
            body: Body::from("{\"move\":\"up\"}"),
            pending: Some(pending),
            complete: false,
        });
        assert!(!tree_caches().snapshot(&game.game.id).unwrap().2);
        assert_eq!(
            axum::body::to_bytes(body, 1024).await.unwrap(),
            "{\"move\":\"up\"}"
        );
        assert!(tree_caches().snapshot(&game.game.id).unwrap().2);
        end(serde_json::to_string(&game).unwrap()).await;
    }

    #[tokio::test]
    async fn superseded_and_ended_requests_cannot_publish_cached_trees() {
        let mut game: Game =
            serde_json::from_str(include_str!("../lib/fixtures/turn33-food.json")).unwrap();
        game.game.id = "superseded-pondering-generation".into();
        game.board.food.clear();
        let _foreground = tree_caches().pause();
        tree_caches().wait_idle().await;
        let body = serde_json::to_string(&game).unwrap();
        let old = decode_state(body.clone()).unwrap();
        let current = decode_state(body.clone()).unwrap();
        assert!(store_search_candidates(&old, searched_candidates(&old)).is_none());
        assert!(store_search_candidates(&current, searched_candidates(&current)).is_some());
        let old_board = old.board.unwrap();
        assert!(!take_search_root(&old, old_board, *old_board.you_id()).1);
        assert!(tree_caches().snapshot(&game.game.id).is_some());

        game.turn -= 1;
        let stale = decode_state(serde_json::to_string(&game).unwrap()).unwrap();
        assert!(store_search_candidates(&stale, searched_candidates(&stale)).is_none());
        end(body.clone()).await;
        // Even recreating the same game ID cannot revive the earlier request lease.
        start(body.clone()).await;
        assert!(store_search_candidates(&current, searched_candidates(&current)).is_none());
        end(body).await;
    }

    #[test]
    fn search_budget_reserves_time_for_the_response() {
        assert_eq!(
            search_budget(500, Duration::ZERO),
            Duration::from_millis(430)
        );
        assert_eq!(
            search_budget(500, Duration::from_millis(75)),
            Duration::from_millis(355)
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

        let value = move_json(body).await;
        let mv = value
            .get("move")
            .and_then(Value::as_str)
            .expect("response must contain a string move");
        assert!(
            ["up", "down", "left", "right"].contains(&mv),
            "expected one of the four lowercase moves, got {mv:?}"
        );
        end(serde_json::to_string(&game).unwrap()).await;
    }

    /// Overlapping same-game requests must both be answered: the Arena times out the
    /// earlier turn and kills the snake when one of them publishes nothing.
    #[tokio::test]
    async fn overlapping_requests_for_one_game_are_both_answered() {
        let mut game: Game =
            serde_json::from_str(include_str!("../lib/fixtures/turn33-food.json")).unwrap();
        game.game.id = "overlapping-same-game".into();
        game.game.timeout = 60;
        game.board.food.clear();
        let body = serde_json::to_string(&game).unwrap();
        let first = tokio::spawn({
            let body = body.clone();
            async move { move_json(body).await }
        });
        // The next turn's request arrives while the first handler is still searching.
        let second = tokio::spawn({
            let body = body.clone();
            async move { move_json(body).await }
        });
        for (name, handle) in [("first", first), ("second", second)] {
            let value = tokio::time::timeout(Duration::from_secs(2), handle)
                .await
                .unwrap_or_else(|_| panic!("{name} request never produced a response"))
                .expect("request task must not panic");
            let mv = value.get("move").and_then(Value::as_str).unwrap_or("");
            assert!(
                ["up", "down", "left", "right"].contains(&mv),
                "{name} response carried {mv:?}"
            );
        }
        end(body).await;
    }

    #[tokio::test]
    async fn move_without_start_initializes_game_state() {
        let mut game: Game = serde_json::from_str(include_str!("../lib/fixtures/turn33-food.json"))
            .expect("valid fixture");
        game.game.id = "missing-start-regression".to_string();
        let body = serde_json::to_string(&game).expect("serialize game");

        let response = move_json(body).await;
        assert!(
            ["up", "down", "left", "right"].contains(&response["move"].as_str().unwrap()),
            "expected a valid move, got {response:?}"
        );
        assert_eq!(
            GAME_STATES.get().unwrap().lock()[&game.game.id].ids[&game.you.id].0,
            0
        );

        end(serde_json::to_string(&game).unwrap()).await;
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
        // Use a searched cache: empty candidates are deliberately not retained.
        let root = Arc::new(Node::new_root(board));
        for _ in 0..100 {
            lib::mcts::search_once(&root, &you, &mut lib::mcts::SearchDepthStats::default());
        }
        tree_caches().insert(
            game.game.id.clone(),
            game.turn,
            SearchTreeCache::after_move(&root, you, root.best_move(you).unwrap()),
        );

        let body = serde_json::to_string(&game).unwrap();
        start(body.clone()).await;
        let cache = tree_caches()
            .take(&game.game.id)
            .expect("late /start preserves cache");
        tree_caches().insert(
            game.game.id.clone(),
            cache.turn,
            SearchTreeCache::after_move(&root, you, root.best_move(you).unwrap()),
        );
        end(body).await;
        assert!(tree_caches().take(&game.game.id).is_none());
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
        let response = move_json(serde_json::to_string(&game).unwrap()).await;
        assert_eq!(response["move"], serde_json::to_value(fallback).unwrap());
        end(serde_json::to_string(&game).unwrap()).await;
    }
}
