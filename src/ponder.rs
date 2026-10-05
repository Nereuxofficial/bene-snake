//! Eight background search threads, shared fairly across games and paused by requests.
use lib::mcts::{SearchDepthStats, SearchTreeCache};
use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex, Weak},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use tracing::{error, info};

pub const TREE_CACHE_TTL: Duration = Duration::from_secs(90);
const PONDER_TIME_LIMIT: Duration = Duration::from_secs(5);
const MAX_CACHED_GAMES: usize = 16;
const PONDER_WORKERS: usize = 8;

pub struct CachedSearch {
    pub turn: i32,
    pub candidates: Arc<SearchTreeCache>,
    pub touched: Instant,
    pub pondered_iterations: u64,
    ready: bool,
}

/// Identity of the cache awaiting delivery of its move response.
pub struct PendingPonder {
    game_id: String,
    candidates: Weak<SearchTreeCache>,
}

#[derive(Default)]
struct State {
    caches: BTreeMap<String, CachedSearch>,
    foreground: usize,
    busy: usize,
    shutdown: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    idle: Notify,
}

pub struct TreeCaches {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

/// RAII also pauses pondering when a request or its search worker is cancelled.
pub struct Foreground(Arc<Shared>);

impl Drop for Foreground {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().foreground -= 1;
        self.0.wake.notify_all();
    }
}

impl TreeCaches {
    pub fn new() -> Self {
        let shared = Arc::new(Shared::default());
        let mut caches = Self {
            shared,
            workers: Vec::with_capacity(PONDER_WORKERS),
        };
        for index in 0..PONDER_WORKERS {
            let worker_shared = Arc::clone(&caches.shared);
            caches.workers.push(
                std::thread::Builder::new()
                    .name(format!("mcts-ponder-{index}"))
                    .spawn(move || run(worker_shared))
                    .expect("create pondering worker"),
            );
        }
        caches
    }

    pub fn pause(&self) -> Foreground {
        self.shared.state.lock().unwrap().foreground += 1;
        Foreground(Arc::clone(&self.shared))
    }

    /// Only iterations already in flight may finish. Await all without blocking
    /// the Tokio executor before matching, publishing, or searching a cached tree.
    pub async fn wait_idle(&self) {
        loop {
            let notified = self.shared.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.shared.state.lock().unwrap().busy == 0 {
                return;
            }
            notified.await;
        }
    }

    pub fn take(&self, game_id: &str) -> Option<CachedSearch> {
        self.shared.state.lock().unwrap().caches.remove(game_id)
    }

    pub fn remove(&self, game_id: &str) {
        self.take(game_id);
    }

    #[cfg(test)]
    pub fn snapshot(&self, game_id: &str) -> Option<(Arc<SearchTreeCache>, u64, bool)> {
        self.shared
            .state
            .lock()
            .unwrap()
            .caches
            .get(game_id)
            .map(|cache| {
                (
                    Arc::clone(&cache.candidates),
                    cache.pondered_iterations,
                    cache.ready,
                )
            })
    }

    pub fn insert(
        &self,
        game_id: String,
        turn: i32,
        candidates: SearchTreeCache,
    ) -> Option<PendingPonder> {
        if candidates.candidate_count() == 0 {
            return None;
        }
        let candidates = Arc::new(candidates);
        let token = PendingPonder {
            game_id: game_id.clone(),
            candidates: Arc::downgrade(&candidates),
        };
        let cached = CachedSearch {
            turn,
            candidates,
            touched: Instant::now(),
            pondered_iterations: 0,
            ready: false,
        };
        // Release retired trees outside the scheduling lock: foreground admission
        // must not wait for recursive destruction of an unrelated game's cache.
        let mut retired = Vec::new();
        {
            let mut state = self.shared.state.lock().unwrap();
            let expired: Vec<_> = state
                .caches
                .iter()
                .filter(|(_, cache)| cache.touched.elapsed() > TREE_CACHE_TTL)
                .map(|(id, _)| id.clone())
                .collect();
            for id in expired {
                retired.push(state.caches.remove(&id));
            }
            if state
                .caches
                .get(&game_id)
                .is_some_and(|cache| cache.turn > turn)
            {
                return None;
            }
            if !state.caches.contains_key(&game_id)
                && state.caches.len() >= MAX_CACHED_GAMES
                && let Some(oldest) = state
                    .caches
                    .iter()
                    .min_by_key(|(_, cache)| cache.touched)
                    .map(|(id, _)| id.clone())
            {
                retired.push(state.caches.remove(&oldest));
            }
            retired.push(state.caches.insert(game_id, cached));
        }
        drop(retired);
        Some(token)
    }

    pub fn response_sent(&self, token: PendingPonder) {
        let mut state = self.shared.state.lock().unwrap();
        if let Some(cache) = state.caches.get_mut(&token.game_id)
            && Weak::ptr_eq(&Arc::downgrade(&cache.candidates), &token.candidates)
        {
            cache.ready = true;
            self.shared.wake.notify_all();
        }
    }
}

impl Drop for TreeCaches {
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().shutdown = true;
        self.shared.wake.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn run(shared: Arc<Shared>) {
    let mut last_game = String::new();
    loop {
        let (game_id, candidates) = {
            let mut state = shared.state.lock().unwrap();
            loop {
                if state.shutdown {
                    return;
                }
                let eligible = |cache: &CachedSearch| {
                    cache.ready && cache.touched.elapsed() < PONDER_TIME_LIMIT
                };
                if state.foreground == 0 {
                    let next = state
                        .caches
                        .iter()
                        .filter(|(_, cache)| eligible(cache))
                        .find(|(id, _)| *id > &last_game)
                        .or_else(|| state.caches.iter().find(|(_, cache)| eligible(cache)))
                        .map(|(id, cache)| (id.clone(), Arc::clone(&cache.candidates)));
                    if let Some(next) = next {
                        state.busy += 1;
                        break next;
                    }
                }
                state = shared.wake.wait(state).unwrap();
            }
        };
        let mut stats = SearchDepthStats::default();
        // Pondering stays off the foreground pool and checks request
        // priority between every iteration, including expensive root preparation.
        let result = catch_unwind(AssertUnwindSafe(|| candidates.ponder_once(&mut stats)));
        let retired = {
            let mut state = shared.state.lock().unwrap();
            if let Some(cache) = state.caches.get_mut(&game_id)
                && Arc::ptr_eq(&cache.candidates, &candidates)
            {
                cache.pondered_iterations += stats.iterations;
                if !matches!(result, Ok(true)) {
                    cache.ready = false;
                }
            }
            // Discard a potentially poisoned tree before waking foreground readers.
            let retired = if result.is_err()
                && state
                    .caches
                    .get(&game_id)
                    .is_some_and(|cache| Arc::ptr_eq(&cache.candidates, &candidates))
            {
                state.caches.remove(&game_id)
            } else {
                None
            };
            state.busy -= 1;
            if state.busy == 0 {
                shared.idle.notify_waiters();
            }
            retired
        };
        if result.is_err() {
            error!(%game_id, "Background exploration failed; discarding its cache");
        }
        drop(retired);
        if !matches!(result, Ok(true)) {
            info!(%game_id, "Background exploration stopped at cache limit or exhausted candidates");
        }
        last_game = game_id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use battlesnake_game_types::{
        types::{YouDeterminableGame, build_snake_id_map},
        wire_representation::Game,
    };
    use lib::mcts::{Node, search_once};

    fn candidates() -> SearchTreeCache {
        let mut game: Game =
            serde_json::from_str(include_str!("../lib/fixtures/turn33-food.json")).unwrap();
        game.board.food.clear();
        let board = game.as_cell_board(&build_snake_id_map(&game)).unwrap();
        let you = *board.you_id();
        let root = Arc::new(Node::new_root(board));
        let mut stats = SearchDepthStats::default();
        for _ in 0..300 {
            search_once(&root, &you, &mut stats);
        }
        let cache = SearchTreeCache::after_move(&root, you, root.best_move(you).unwrap());
        assert!(cache.candidate_count() > 0);
        cache
    }

    async fn wait_for_iterations(caches: &TreeCaches, ids: &[&str]) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let advanced = {
                    let state = caches.shared.state.lock().unwrap();
                    ids.iter()
                        .all(|id| state.caches[*id].pondered_iterations > 0)
                };
                if advanced {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("background worker must advance every ready game");
    }

    #[tokio::test]
    async fn pondering_waits_for_delivery_and_yields_to_all_foreground_requests() {
        let caches = TreeCaches::new();
        let pending = caches.insert("game".into(), 3, candidates()).unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(
            caches.shared.state.lock().unwrap().caches["game"].pondered_iterations,
            0
        );
        let first = caches.pause();
        let second = caches.pause();
        caches.wait_idle().await;
        caches.response_sent(pending);
        drop(first);
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(
            caches.shared.state.lock().unwrap().caches["game"].pondered_iterations,
            0
        );
        drop(second);
        wait_for_iterations(&caches, &["game"]).await;

        let foreground = caches.pause();
        tokio::time::timeout(Duration::from_millis(100), caches.wait_idle())
            .await
            .expect("a foreground request must quiesce pondering promptly");
        let iterations = caches.shared.state.lock().unwrap().caches["game"].pondered_iterations;
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(
            caches.shared.state.lock().unwrap().caches["game"].pondered_iterations,
            iterations
        );
        caches.remove("game");
        drop(foreground);
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(caches.take("game").is_none());
    }

    #[tokio::test]
    async fn games_share_background_workers_and_detached_caches_stop_growing() {
        let caches = TreeCaches::new();
        let foreground = caches.pause();
        for id in ["a", "b"] {
            let pending = caches.insert(id.into(), 3, candidates()).unwrap();
            caches.response_sent(pending);
        }
        drop(foreground);
        wait_for_iterations(&caches, &["a", "b"]).await;
        let foreground = caches.pause();
        caches.wait_idle().await;
        let detached = caches.take("a").unwrap();
        let visits = detached.candidates.retained_visits();
        let mut stats = SearchDepthStats::default();
        drop(foreground);
        tokio::time::sleep(Duration::from_millis(10)).await;
        // A detached tree has no scheduler job left; only explicit work advances it.
        assert_eq!(detached.candidates.retained_visits(), visits);
        assert!(detached.candidates.ponder_once(&mut stats));
        assert_eq!(stats.iterations, 1);
        assert_eq!(detached.candidates.retained_visits(), visits + 1);
        assert!(caches.take("a").is_none());
    }

    #[tokio::test]
    async fn stale_delivery_cannot_activate_replaced_or_ended_game() {
        let caches = TreeCaches::new();
        let foreground = caches.pause();
        let old = caches.insert("game".into(), 3, candidates()).unwrap();
        let current = caches.insert("game".into(), 4, candidates()).unwrap();
        caches.response_sent(old);
        assert!(!caches.shared.state.lock().unwrap().caches["game"].ready);
        assert!(caches.insert("game".into(), 2, candidates()).is_none());
        assert_eq!(caches.shared.state.lock().unwrap().caches["game"].turn, 4);
        caches.remove("game");
        caches.response_sent(current);
        drop(foreground);
        assert!(caches.take("game").is_none());
    }

    #[tokio::test]
    async fn abandoned_games_expire_and_cache_count_is_bounded() {
        let caches = TreeCaches::new();
        let foreground = caches.pause();
        let pending = caches.insert("expired".into(), 3, candidates()).unwrap();
        caches.response_sent(pending);
        caches
            .shared
            .state
            .lock()
            .unwrap()
            .caches
            .get_mut("expired")
            .unwrap()
            .touched = Instant::now() - PONDER_TIME_LIMIT;
        drop(foreground);
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(
            caches.shared.state.lock().unwrap().caches["expired"].pondered_iterations,
            0
        );

        let foreground = caches.pause();
        caches
            .shared
            .state
            .lock()
            .unwrap()
            .caches
            .get_mut("expired")
            .unwrap()
            .touched = Instant::now() - TREE_CACHE_TTL - Duration::from_secs(1);
        for i in 0..=MAX_CACHED_GAMES {
            caches.insert(format!("game-{i:02}"), 3, candidates());
        }
        let state = caches.shared.state.lock().unwrap();
        assert!(!state.caches.contains_key("expired"));
        assert!(!state.caches.contains_key("game-00"));
        assert_eq!(state.caches.len(), MAX_CACHED_GAMES);
        drop(state);
        drop(foreground);
    }

    #[tokio::test]
    async fn cancelling_a_foreground_request_resumes_background_work() {
        let caches = Arc::new(TreeCaches::new());
        let pending = caches.insert("game".into(), 3, candidates()).unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let request_caches = Arc::clone(&caches);
        let request = tokio::spawn(async move {
            let _foreground = request_caches.pause();
            request_caches.wait_idle().await;
            ready_tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        ready_rx.await.unwrap();
        caches.response_sent(pending);
        assert_eq!(caches.snapshot("game").unwrap().1, 0);
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        wait_for_iterations(&caches, &["game"]).await;
    }
}
