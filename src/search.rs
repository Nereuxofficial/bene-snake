//! Prepare a safe emergency response before workers own the search tree.
use crate::game_state::ResponseMoves;
use battlesnake_game_types::types::{Move, SnakeId};
use lib::{
    mcts::{Node, SearchTreeCache, mcts_search_with_publish},
    tactical::{Limits, RootFilter},
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Instant,
};

pub type SearchResult = Option<(Move, CacheDelivery)>;

type RetainCache = Box<dyn FnOnce(SearchTreeCache) + Send>;

#[derive(Default)]
struct DeliveryState {
    delivered: bool,
    abandoned: bool,
    cache: Option<SearchTreeCache>,
    retain: Option<RetainCache>,
}

/// The response owns only this small rendezvous, never the searched root. Cache
/// construction and destruction stay on blocking threads, even if delivery wins
/// the race with snapshot completion or the response is dropped unsent.
pub struct CacheDelivery {
    shared: Arc<Mutex<DeliveryState>>,
    runtime: tokio::runtime::Handle,
}

struct CacheCompletion {
    shared: Arc<Mutex<DeliveryState>>,
    runtime: tokio::runtime::Handle,
}

impl CacheDelivery {
    fn new() -> (Self, CacheCompletion) {
        let shared = Arc::new(Mutex::new(DeliveryState::default()));
        let runtime = tokio::runtime::Handle::current();
        (
            Self {
                shared: Arc::clone(&shared),
                runtime: runtime.clone(),
            },
            CacheCompletion { shared, runtime },
        )
    }

    #[cfg(test)]
    pub(crate) fn ready_for_test(cache: SearchTreeCache) -> Self {
        let (delivery, completion) = Self::new();
        completion.complete(cache);
        delivery
    }

    pub fn on_delivery(&self, retain: impl FnOnce(SearchTreeCache) + Send + 'static) {
        self.shared.lock().unwrap().retain = Some(Box::new(retain));
    }

    pub fn delivered(&self) {
        let mut state = self.shared.lock().unwrap();
        state.delivered = true;
        if let Some(cache) = state.cache.take() {
            let retain = state.retain.take();
            self.runtime.spawn_blocking(move || {
                if let Some(retain) = retain {
                    retain(cache);
                }
            });
        }
    }
}

impl Drop for CacheDelivery {
    fn drop(&mut self) {
        let mut state = self.shared.lock().unwrap();
        if !state.delivered {
            state.abandoned = true;
            state.retain = None;
            if let Some(cache) = state.cache.take() {
                self.runtime.spawn_blocking(move || drop(cache));
            }
        }
    }
}

impl CacheCompletion {
    fn complete(self, cache: SearchTreeCache) {
        let mut state = self.shared.lock().unwrap();
        if state.abandoned {
            drop(state);
            drop(cache); // On the search worker, outside the delivery lock.
            return;
        }
        if state.delivered {
            let retain = state.retain.take();
            self.runtime.spawn_blocking(move || {
                if let Some(retain) = retain {
                    retain(cache);
                }
            });
        } else {
            state.cache = Some(cache);
        }
    }
}

pub struct PreparedSearch {
    root: Arc<Node>,
    you: SnakeId,
    filter: Option<RootFilter>,
    moves: ResponseMoves,
}

impl PreparedSearch {
    /// The caller must pause pondering and wait for the root to become idle first.
    /// Preparation is charged to the request's existing deadline. Incomplete
    /// tactical passes retain their last complete horizon; a panic leaves the
    /// tactical filter absent and preserves the wire fallback if selection fails.
    pub fn new(root: Arc<Node>, you: SnakeId, wire_moves: &ResponseMoves, continued: bool) -> Self {
        let filter = catch_unwind(AssertUnwindSafe(|| {
            root.tactical_root_filter_with_continuation(you, &Limits::default(), continued)
        }))
        .ok();
        let fallback = catch_unwind(AssertUnwindSafe(|| {
            root.guarded_fallback(you, filter.as_ref(), wire_moves.acceptable)
        }))
        .ok()
        .flatten()
        .unwrap_or(wire_moves.fallback);
        Self {
            root,
            you,
            filter,
            moves: ResponseMoves {
                fallback,
                acceptable: wire_moves.acceptable,
            },
        }
    }

    pub fn fallback(&self) -> Move {
        self.moves.fallback
    }

    pub fn tactical_filter_applied(&self) -> bool {
        self.filter.as_ref().is_some_and(|filter| filter.applied)
    }

    pub fn food_target(&self) -> Option<u8> {
        self.root.food_target()
    }

    pub fn food_race(&self) -> Option<&lib::food_race::Assessment> {
        self.root.root_food_race(self.you)
    }

    pub fn run(
        self,
        stop: Arc<AtomicBool>,
        trace: &crate::telemetry::MoveTrace,
        publish: impl FnOnce(SearchResult) -> bool,
    ) {
        mcts_search_with_publish(Arc::clone(&self.root), &self.you, stop, || {
            // This callback runs only after every search worker has joined.
            let joined_at = Instant::now();
            let selection_started = Instant::now();
            let chosen = catch_unwind(AssertUnwindSafe(|| {
                self.root
                    .best_move_with_root_filter(self.you, self.filter.as_ref())
            }))
            .ok()
            .flatten()
            .filter(|chosen| self.moves.acceptable[chosen.as_index()]);
            let selection_elapsed = selection_started.elapsed();
            // Capture timing at the boundary, but emit stage logs only after
            // sending the move. Logging is optional publication-tail work too.
            let publish = |result| {
                let accepted = publish(result);
                trace.workers_stopped(joined_at);
                trace.search_stage("selection_completed", selection_elapsed);
                accepted
            };
            let Some(chosen) = chosen else {
                publish(None);
                return;
            };
            publish_then_snapshot(chosen, trace, publish, || {
                SearchTreeCache::after_move(&self.root, self.you, chosen)
            });
        });
    }
}

fn publish_then_snapshot(
    chosen: Move,
    trace: &crate::telemetry::MoveTrace,
    publish: impl FnOnce(SearchResult) -> bool,
    snapshot: impl FnOnce() -> SearchTreeCache,
) {
    let (delivery, completion) = CacheDelivery::new();
    if !publish(Some((chosen, delivery))) {
        return;
    }
    let started = Instant::now();
    let snapshot = catch_unwind(AssertUnwindSafe(snapshot));
    trace.search_stage(
        if snapshot.is_ok() {
            "snapshot_completed"
        } else {
            "snapshot_panicked"
        },
        started.elapsed(),
    );
    if let Ok(cache) = snapshot {
        completion.complete(cache);
    }
}

#[cfg(test)]
pub(super) fn validated_search_result(
    worker_finished: bool,
    moves: &ResponseMoves,
    publish: impl FnOnce() -> SearchResult,
) -> SearchResult {
    if !worker_finished {
        return None;
    }
    catch_unwind(AssertUnwindSafe(publish))
        .ok()
        .flatten()
        .filter(|(chosen, _)| moves.acceptable[chosen.as_index()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use battlesnake_game_types::{
        compact_representation::standard::CellBoard4Snakes11x11, types::build_snake_id_map,
        wire_representation::Game,
    };

    fn cache() -> SearchTreeCache {
        let game: Game =
            serde_json::from_str(include_str!("../lib/fixtures/turn33-food.json")).unwrap();
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let root = Arc::new(Node::new_root(game.as_cell_board(&ids).unwrap()));
        SearchTreeCache::after_move(&root, you, Move::Up)
    }

    #[tokio::test]
    async fn cache_delivery_requires_consumption_in_both_completion_orders() {
        for snapshot_first in [false, true] {
            let (delivery, completion) = CacheDelivery::new();
            let (retained, receive) = tokio::sync::oneshot::channel();
            delivery.on_delivery(move |_| {
                let _ = retained.send(());
            });
            if snapshot_first {
                completion.complete(cache());
                assert!(delivery.shared.lock().unwrap().cache.is_some());
                delivery.delivered();
            } else {
                delivery.delivered();
                completion.complete(cache());
            }
            tokio::time::timeout(std::time::Duration::from_secs(1), receive)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn abandoned_response_discards_early_and_late_snapshots() {
        for snapshot_first in [false, true] {
            let (delivery, completion) = CacheDelivery::new();
            let (retained, receive) = tokio::sync::oneshot::channel();
            delivery.on_delivery(move |_| {
                let _ = retained.send(());
            });
            if snapshot_first {
                completion.complete(cache());
                drop(delivery);
            } else {
                drop(delivery);
                completion.complete(cache());
            }
            assert!(receive.await.is_err());
        }
    }

    #[tokio::test]
    async fn publication_precedes_snapshot_and_survives_snapshot_panic() {
        for panic_snapshot in [false, true] {
            let (published, receive) = tokio::sync::oneshot::channel();
            let (entered, snapshot_entered) = tokio::sync::oneshot::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            let worker = tokio::task::spawn_blocking(move || {
                publish_then_snapshot(
                    Move::Right,
                    &crate::telemetry::MoveTrace::current(),
                    |result| published.send(result).is_ok(),
                    || {
                        entered.send(()).unwrap();
                        blocked.recv().unwrap();
                        if panic_snapshot {
                            panic!("injected optional snapshot panic");
                        }
                        cache()
                    },
                );
            });
            snapshot_entered.await.unwrap();
            let (chosen, delivery) = receive.await.unwrap().unwrap();
            assert_eq!(chosen, Move::Right);
            assert!(!worker.is_finished());
            release.send(()).unwrap();
            worker.await.unwrap();
            drop(delivery);
        }
    }

    #[tokio::test]
    async fn response_abandoned_during_snapshot_never_retains_cache() {
        let (published, receive) = tokio::sync::oneshot::channel();
        let (entered, snapshot_entered) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let worker = tokio::task::spawn_blocking(move || {
            publish_then_snapshot(
                Move::Right,
                &crate::telemetry::MoveTrace::current(),
                |result| published.send(result).is_ok(),
                || {
                    entered.send(()).unwrap();
                    blocked.recv().unwrap();
                    cache()
                },
            );
        });
        snapshot_entered.await.unwrap();
        let (_, delivery) = receive.await.unwrap().unwrap();
        let (retained, receive_retained) = tokio::sync::oneshot::channel();
        delivery.on_delivery(move |_| {
            let _ = retained.send(());
        });
        drop(delivery);
        release.send(()).unwrap();
        worker.await.unwrap();
        assert!(receive_retained.await.is_err());
    }

    #[tokio::test]
    async fn body_error_followed_by_eof_never_authorises_cache_retention() {
        use axum::body::{Body, Bytes, HttpBody};
        use std::{
            pin::Pin,
            task::{Context, Poll, Waker},
        };
        struct ErrorThenEnd(bool);
        impl HttpBody for ErrorThenEnd {
            type Data = Bytes;
            type Error = std::io::Error;
            fn poll_frame(
                mut self: Pin<&mut Self>,
                _: &mut Context<'_>,
            ) -> Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
                if self.0 {
                    Poll::Ready(None)
                } else {
                    self.0 = true;
                    Poll::Ready(Some(Err(std::io::Error::other("injected body error"))))
                }
            }
            fn is_end_stream(&self) -> bool {
                self.0
            }
        }
        let (delivery, completion) = CacheDelivery::new();
        let (retained, receive) = tokio::sync::oneshot::channel();
        delivery.on_delivery(move |_| {
            let _ = retained.send(());
        });
        completion.complete(cache());
        let mut response = crate::DeferredPonderResponse {
            body: Body::new(ErrorThenEnd(false)),
            delivery,
            complete: false,
            failed: false,
        };
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(
            Pin::new(&mut response).poll_frame(&mut cx),
            Poll::Ready(Some(Err(_)))
        ));
        assert!(matches!(
            Pin::new(&mut response).poll_frame(&mut cx),
            Poll::Ready(None)
        ));
        drop(response);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), receive)
                .await
                .unwrap()
                .is_err()
        );
    }

    #[tokio::test]
    async fn rejected_publication_skips_snapshot() {
        publish_then_snapshot(
            Move::Up,
            &crate::telemetry::MoveTrace::current(),
            |_| false,
            || panic!("receiver gone: snapshot must not run"),
        );
    }

    #[test]
    fn guarded_fallback_intersects_wire_acceptability_before_selection() {
        let game: Game =
            serde_json::from_str(include_str!("fixtures/68adaa6a-turn141.json")).unwrap();
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let root = Node::new_root(board);
        // Left is wire-legal but a proved self-trap; Right is the only guarded
        // candidate. A wire veto on Right must never reintroduce Left.
        assert_eq!(
            root.guarded_fallback(you, None, [true; 4]),
            Some(Move::Right)
        );
        assert_eq!(
            root.guarded_fallback(you, None, [false, false, true, false]),
            None
        );
    }
}
