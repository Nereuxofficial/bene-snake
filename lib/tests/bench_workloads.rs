#![cfg(feature = "bench")]
#[path = "../benches/support/mod.rs"]
mod support;
use battlesnake_game_types::types::{
    HealthGettableGame, LengthGettableGame, SimulableGame, VictorDeterminableGame,
};
use std::collections::BTreeSet;
#[test]
fn fixtures_cover_live_multiplayer_and_late_food_decisions() {
    let cases = support::fixtures();
    assert_eq!(
        cases.iter().map(|f| &f.name).collect::<BTreeSet<_>>().len(),
        cases.len()
    );
    assert_eq!(
        cases
            .iter()
            .map(|f| f.board.alive_snake_count())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([2, 3, 4])
    );
    assert!(cases.iter().any(|f| f.board.get_health(&f.you) < 30));
    assert!(cases.iter().any(|f| f.board.get_length(&f.you) > 20));
    for f in cases {
        assert_eq!(f.actions.len(), 64);
        for action in f.actions {
            assert_eq!(action.len(), f.board.alive_snake_count());
            let singleton: arrayvec::ArrayVec<_, 4> =
                action.iter().map(|(id, mv)| (*id, [*mv])).collect();
            assert_eq!(
                f.board.simulate_single_action(&action),
                f.board
                    .simulate_with_moves(&singleton)
                    .next()
                    .expect("one action")
            );
        }
    }
}
#[test]
fn warm_search_samples_repeat_the_same_tree_and_rng_workload() {
    for f in support::fixtures() {
        let mut a = support::Search::new(&f, support::WARMUP_ITERATIONS);
        let mut b = support::Search::new(&f, support::WARMUP_ITERATIONS);
        assert_eq!(a.stats.iterations, support::WARMUP_ITERATIONS);
        assert_eq!(a.root.visits(), support::WARMUP_ITERATIONS as u32);
        a.run(f.you, support::SEARCH_BATCH);
        b.run(f.you, support::SEARCH_BATCH);
        assert_eq!(
            a.root.visits(),
            (support::WARMUP_ITERATIONS + support::SEARCH_BATCH) as u32
        );
        assert_eq!(a.root.best_move(f.you), b.root.best_move(f.you));
        assert_eq!(a.stats.max_tree_depth, b.stats.max_tree_depth);
        assert_eq!(
            a.stats.rollout_depth_limit_hits,
            b.stats.rollout_depth_limit_hits
        );
    }
}
