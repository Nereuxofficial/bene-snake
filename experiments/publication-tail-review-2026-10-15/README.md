# Publication tail: implementation and independent review

Implemented by a subagent, then independently reviewed and corrected by the parent. Changes are local, uncommitted and undeployed. The previously deployed revision remains unchanged.

## Runtime behavior

After all foreground workers join, `PreparedSearch` selects and validates the move against the precomputed tactical filter and wire-acceptable directions. It sends that move through the existing oneshot **before** optional `SearchTreeCache::after_move` snapshot construction. A snapshot panic or slow snapshot cannot revoke or delay an already sent selection. Rejected publication skips snapshot work. Optional snapshot/destruction remains on blocking threads; the response carries a small rendezvous ticket rather than the searched root.

Cache construction and body consumption can finish in either order. Cache insertion only occurs after the response body has been consumed, and rechecks the original request generation before insertion. Dropped or failed bodies abandon their cache; `/end`, a newer request, or recreation of the same game ID invalidates the old lease. Missing a cache because the next turn arrived first is acceptable; using stale state is not. Retired cache destruction is moved outside both the generation and scheduling locks. The final response still does not claim Arena acknowledgement.

Telemetry now records request stop, worker join relative to the stop instant, final selection duration, move publication duration, snapshot duration/completion/panic and deadline lateness, correlated by game/turn/request. The parent moved join/selection stage emission after the move send while retaining the actual join timestamp, so those stage logs do not extend the move's critical path. No stop request before the join remains `None`, rather than an invented zero-duration stop.

The request's existing search/delivery budget and safe fallback remain in force. This does not implement the separate single-candidate fast path or change opponent/rollout policies. Publication remains conditional on workers having stopped; the handler never inspects a mutating tree.

## Review fixes

1. Added a failing Err-then-EOF body regression. The first implementation reset `complete` on an error, then could set it true on a subsequent EOF poll, incorrectly retaining a failed response's cache. The parent added a permanent `failed` flag; the regression now passes.
2. Removed the retired test-only `PonderResponse` wrapper. The consumed/abandoned response integration test now exercises the actual production `DeferredPonderResponse`, rather than preserving passing tests around unused behavior.
3. Verified publication ordering, cache ownership/drop paths, generation leases and lock release. Deferred cache retirees are destroyed after generation/scheduler guards release. Added timing-field regression for identity, configured deadlines and absent stop information.

## Verification and limits

Deterministic barrier tests prove the move is published while the snapshot builder remains blocked, including an injected later snapshot panic. Further tests cover rejected receivers, both snapshot/body completion orders, abandonment before/after/during snapshot, failed-body EOF handling, `/end` plus recreated IDs, newer generations, guarded fallback, actual body-consumption gating and downtime cache reuse.

Final combined run: **108 library unit tests passed, one existing diagnostic ignored; 44 server tests passed.** Clippy for library and server across all targets passed with `-D warnings`; `git diff --check` passed. Logs are stored here. This demonstrates ordering and lifecycle correctness, not a measured number of Arena timeouts prevented or a deployment-CPU latency improvement. Worker scheduling/join cost remains on the publication path and is now observable.

The separate enclosure evaluator and its final replay evidence are documented in `../enclosure-2026-10-15/README.md`. It remains opt-in because the representative equal-time comparison found no decision benefit and a measurable throughput cost.
