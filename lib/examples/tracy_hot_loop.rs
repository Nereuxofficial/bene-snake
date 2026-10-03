//! Release-mode seeded Arena search workload for Tracy.
#[path = "../benches/support/mod.rs"]
mod support;
use std::time::{Duration, Instant};
fn main() {
    let client = tracy_client::Client::start();
    client.set_thread_name("MCTS hot loop");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !tracy_client::Client::is_connected() {
        assert!(
            Instant::now() < deadline,
            "Tracy did not connect within 60 seconds"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let fixtures = support::fixtures();
    let duration = Duration::from_secs(
        std::env::args()
            .nth(1)
            .map(|s| s.parse().expect("seconds"))
            .unwrap_or(15),
    );
    let start = Instant::now();
    let mut total = 0u64;
    while start.elapsed() < duration {
        for fixture in &fixtures {
            let span = client.clone().span_alloc(
                Some(&fixture.name),
                "Arena search batch",
                file!(),
                line!(),
                0,
            );
            let mut search = support::Search::new(fixture, 0);
            search.run(fixture.you, support::SEARCH_BATCH);
            std::hint::black_box(search.root.best_move(fixture.you));
            total += search.stats.iterations;
            drop(search);
            drop(span);
            client.frame_mark();
        }
    }
    eprintln!(
        "Captured {total} search iterations across {} fixtures in {:?}",
        fixtures.len(),
        start.elapsed()
    );
    std::thread::sleep(Duration::from_secs(2));
}
