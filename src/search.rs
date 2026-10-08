//! Prepare a safe emergency response before workers own the search tree.
use crate::game_state::ResponseMoves;
use battlesnake_game_types::types::{Move, SnakeId};
use lib::{
    mcts::{Node, SearchTreeCache, mcts_search_with_publish},
    tactical::{Limits, RootFilter},
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, atomic::AtomicBool},
};

pub type SearchResult = Option<(Move, SearchTreeCache)>;

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
    pub fn new(root: Arc<Node>, you: SnakeId, wire_moves: &ResponseMoves) -> Self {
        let filter = catch_unwind(AssertUnwindSafe(|| {
            root.tactical_root_filter(you, &Limits::default())
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

    pub fn run(self, stop: Arc<AtomicBool>, publish: impl FnOnce(SearchResult)) {
        mcts_search_with_publish(Arc::clone(&self.root), &self.you, stop, || {
            // Publication runs only after all workers stop mutating the tree.
            let result = validated_search_result(true, &self.moves, || {
                let chosen = self
                    .root
                    .best_move_with_root_filter(self.you, self.filter.as_ref())?;
                Some((
                    chosen,
                    SearchTreeCache::after_move(&self.root, self.you, chosen),
                ))
            });
            publish(result);
        });
    }
}

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
