# bene-snake

# Running the project
After [installing Rust](https://rustup.rs), run the following command in the project directory:
```
cargo run --release
```

Move requests search with 12 workers. After a move response is delivered, eight
shared background workers continue searching the retained trees for possible
opponent replies. They give more work to replies seen more often during the move
search and share time across active games. Incoming move requests pause background
work before accessing a tree; an exact next-turn board match carries
the added visits into the foreground search.

Background work stops after five seconds or when the cache reaches 20,000 retained
visits. The server retains at most 32 replies per game and 16 games. Trees with a
changed own length, eliminated snake, or completed game are skipped; `/end` removes
the cache. Move logs report `pondered_iterations`, `reused`, and `carried_visits`.
