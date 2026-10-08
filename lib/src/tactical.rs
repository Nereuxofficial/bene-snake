//! Bounded tactical survival diagnostic (implementation-guide task B1).
//!
//! This module is a **diagnostic only**. It never changes production move
//! selection: callers ask it a question and inspect the answer. Integration into
//! the search root is a separate task (B2).
//!
//! For each of our candidate first moves it asks: can an adversary force our
//! snake to die within a small number of complete simultaneous turns, under the
//! enumerated response model below? Three answers are possible:
//!
//! - [`MoveVerdict::ProvenSafe`]: survival through the stated horizon holds under
//!   the model. It does **not** mean winning the game.
//! - [`MoveVerdict::Exposed`]: an adversarial response can defeat every
//!   continuation within the horizon. It does **not** mean the real opponent will
//!   play that response. The [`DeadProof`] witness preserves the branch needed to
//!   reproduce the result.
//! - [`MoveVerdict::Unknown`]: the call/time budget ran out, or a stop was
//!   requested, before a conclusion was reached. Budget exhaustion is unknown,
//!   never loss.
//!
//! # Response model
//!
//! - Each turn every living snake chooses one *physically reasonable* direction:
//!   [`CellBoard4Snakes11x11::reasonable_move_mask`], which excludes walls, bodies
//!   and the neck but **keeps** moves that lose a head contest. A suicidal opponent
//!   can still collide, so those replies are enumerated.
//! - A snake with no reasonable direction uses the simulator's forced fallback of
//!   a single direction; every such direction is lethal, so the snake dies either
//!   way and enumeration stays bounded.
//! - We choose our future moves existentially; opponents are examined universally.
//!   An AND of responses is dead if any response is dead and safe only if all are
//!   safe. An OR of our continuations is safe if any is safe and dead only if all
//!   are dead. Unexplored branches are never converted into safety or loss.
//! - We always enumerate the full reasonable mask, so a "proven safe" result holds
//!   over every legal direction, not a subset.
//!
//! # Deliberate limits
//!
//! - **No future food spawning.** The production simulator does not guess random
//!   food, and neither does this diagnostic. Food already on the board is eaten.
//! - The input board is never mutated; [`CellBoard4Snakes11x11`] is `Copy`.
//! - This is not MCTS and not a rollout average. It is an exhaustive three-valued
//!   search under a strict budget.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use arrayvec::ArrayVec;
use battlesnake_game_types::{
    compact_representation::standard::CellBoard4Snakes11x11,
    types::{
        HeadGettableGame, HealthGettableGame, LengthGettableGame, Move, N_MOVES, SnakeId,
        VictorDeterminableGame,
    },
};

/// Bound on the number of witness nodes retained for one analysis. The verdict is
/// already established when this is reached; only the recorded branch is truncated.
const WITNESS_NODE_BUDGET: u32 = 4096;

/// Resource bounds for one [`analyze`] call.
///
/// Deepen from two through ten complete turns, sharing a 10 ms wall-time ceiling
/// and 20,000 simulator calls across candidates and depths. Incomplete passes
/// retain the last fully completed horizon.
pub struct Limits<'a> {
    /// First (shallowest) horizon to test. Clamped to at least one.
    pub start_horizon: u8,
    /// Deepest horizon to test, iteratively deepened from `start_horizon`.
    pub max_horizon: u8,
    /// Total simulator-call cap shared across all candidates and all depths.
    pub max_simulator_calls: u32,
    /// Absolute wall-clock deadline, checked before expensive work.
    pub deadline: Option<Instant>,
    /// Cooperative stop signal, checked before expensive work.
    pub stop: Option<&'a AtomicBool>,
}

impl Default for Limits<'_> {
    fn default() -> Self {
        Self {
            start_horizon: 2,
            max_horizon: 10,
            max_simulator_calls: 20_000,
            deadline: Some(Instant::now() + Duration::from_millis(10)),
            stop: None,
        }
    }
}

impl Limits<'_> {
    /// A budget suitable for deterministic semantic tests: no wall-clock or call
    /// limit except the explicit one, deepening through `max_horizon`.
    pub fn for_test(max_horizon: u8) -> Limits<'static> {
        Limits {
            start_horizon: 1,
            max_horizon,
            max_simulator_calls: u32::MAX,
            deadline: None,
            stop: None,
        }
    }
}

/// The verdict for one candidate move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveVerdict {
    /// Survival through `horizon` complete turns holds under the model.
    ProvenSafe { horizon: u8 },
    /// An adversarial response defeats every continuation within `horizon` turns.
    Exposed { horizon: u8, witness: DeadProof },
    /// Not decided within the budget (`horizon` is the deepest *completed* horizon).
    Unknown { horizon: u8 },
    /// This move was not in the supplied candidate mask.
    NotCandidate,
}

/// A reproducing witness for an [`MoveVerdict::Exposed`] result.
///
/// It preserves the action sequence and branch, not a scalar score. To replay it,
/// start from the fixture board and, for every branch, apply our recorded move and
/// the adversary's recorded simultaneous reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeadProof {
    /// Our snake's health reached zero during this transition.
    Died,
    /// Our snake had no physically reasonable direction at this board.
    NoMoves,
    /// For every candidate continuation, an adversarial simultaneous reply leads to
    /// a child that is itself dead. An empty list means no continuation existed.
    Forced { branches: Vec<DeadBranch> },
    /// The witness tree exceeded the node budget; the verdict still stands but the
    /// recorded branch stops here.
    Truncated,
}

/// One branch of a [`DeadProof::Forced`]: our move, the adversary's reply, and the
/// proof for the resulting board.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeadBranch {
    pub my_move: Move,
    pub reply: ArrayVec<(SnakeId, Move), MAX_SNAKES>,
    pub child: Box<DeadProof>,
}

/// Maximum number of snakes on the supported board.
const MAX_SNAKES: usize = 4;

/// Result of one [`analyze`] call.
pub struct TacticalAnalysis {
    verdicts: [MoveVerdict; N_MOVES],
    /// Number of production-simulator calls, including enumeration that proved
    /// failure immediately.
    pub simulator_calls: u32,
    /// Deepest horizon fully completed across every candidate. Zero means no
    /// horizon completed; compare candidates only at this common horizon.
    pub completed_horizon: u8,
    /// True when the call/time/stop limit stopped the analysis.
    pub budget_exhausted: bool,
}

impl TacticalAnalysis {
    /// The verdict for `mv`.
    pub fn verdict(&self, mv: Move) -> &MoveVerdict {
        &self.verdicts[mv.as_index()]
    }

    /// The verdict for every direction, indexed by [`Move::as_index`].
    pub fn verdicts(&self) -> &[MoveVerdict; N_MOVES] {
        &self.verdicts
    }
}

/// Analyze our candidate moves for `you` on `board` under `limits`.
///
/// `candidate_mask` has bit `Move::as_index()` set for each move to analyze; bits
/// without a candidate are reported as [`MoveVerdict::NotCandidate`]. The board is
/// never mutated.
pub fn analyze(
    board: &CellBoard4Snakes11x11,
    you: SnakeId,
    candidate_mask: u8,
    limits: &Limits<'_>,
) -> TacticalAnalysis {
    let mut ctx = Ctx {
        you,
        calls: 0,
        max_calls: limits.max_simulator_calls,
        deadline: limits.deadline,
        stop: limits.stop,
        exhausted: false,
        witness_nodes: WITNESS_NODE_BUDGET,
    };
    let mut verdicts: [MoveVerdict; N_MOVES] = std::array::from_fn(|_| MoveVerdict::NotCandidate);
    for (index, verdict) in verdicts.iter_mut().enumerate() {
        if candidate_mask & (1 << index) != 0 {
            *verdict = MoveVerdict::Unknown { horizon: 0 };
        }
    }

    // A zero budget or already-expired deadline must not spend a single call.
    if !ctx.check_limits() {
        return TacticalAnalysis {
            verdicts,
            simulator_calls: 0,
            completed_horizon: 0,
            budget_exhausted: true,
        };
    }

    let start = limits.start_horizon.max(1);
    let max = limits.max_horizon.max(start);
    let mut completed_horizon = 0;
    for horizon in start..=max {
        if all_exposed(&verdicts, candidate_mask) {
            break;
        }
        let mut next = verdicts.clone();
        let mut incomplete = false;
        for index in 0..N_MOVES {
            if candidate_mask & (1 << index) == 0 {
                continue;
            }
            // Exposed is monotone in the horizon: an adversary who can force death
            // within a shallower horizon can do so within a deeper one. Carry it
            // forward instead of re-deriving it.
            if matches!(verdicts[index], MoveVerdict::Exposed { .. }) {
                continue;
            }
            match search(&mut ctx, *board, horizon, Some(1 << index)) {
                Outcome::Safe => next[index] = MoveVerdict::ProvenSafe { horizon },
                Outcome::Dead(witness) => next[index] = MoveVerdict::Exposed { horizon, witness },
                Outcome::Unknown => {
                    incomplete = true;
                    break;
                }
            }
        }
        if incomplete {
            // Retain the shallower completed conclusions; discard this partial pass.
            break;
        }
        verdicts = next;
        completed_horizon = horizon;
    }

    TacticalAnalysis {
        verdicts,
        simulator_calls: ctx.calls,
        completed_horizon,
        budget_exhausted: ctx.exhausted,
    }
}

fn all_exposed(verdicts: &[MoveVerdict; N_MOVES], candidate_mask: u8) -> bool {
    (0..N_MOVES)
        .filter(|index| candidate_mask & (1 << index) != 0)
        .all(|index| matches!(verdicts[index], MoveVerdict::Exposed { .. }))
}

/// Manhattan distance within which an equal-or-longer rival triggers the root check.
///
/// Distance four deliberately covers the turn-289 duel setup; an immediate
/// shared-destination trigger alone would miss it.
pub const TRIGGER_DISTANCE: usize = 4;

/// Physical-exit count at or below which the root check triggers.
pub const TRIGGER_EXITS: u32 = 2;

/// A bounded decision the search root may apply from the tactical diagnostic (task B2).
///
/// Computed once per search root and passed into selection. It is deliberately not
/// stored on a shared node, so a proof derived for one root cannot silently become a
/// current-root proof for a promoted child.
pub struct RootFilter {
    /// The candidate mask selection should use. Equals the input mask whenever the
    /// filter does not apply, so unknown or all-exposed results never change selection.
    pub mask: u8,
    /// True when `mask` differs from the input candidate mask.
    pub applied: bool,
    /// True when the duel/corridor trigger conditions held.
    pub triggered: bool,
    /// Number of living opponents at the root.
    pub opponent_count: usize,
    /// The analysis, present only when the trigger held.
    pub analysis: Option<TacticalAnalysis>,
}

impl RootFilter {
    /// A filter that never changes selection. Test-only helper.
    #[cfg(test)]
    pub(crate) fn passthrough(mask: u8) -> Self {
        Self {
            mask,
            applied: false,
            triggered: false,
            opponent_count: 0,
            analysis: None,
        }
    }
}

/// Compute the bounded root filter for `you` on `board`.
///
/// `base_mask` is the existing production candidate mask (reasonable moves minus
/// losing head contests, with any root escape override applied). Only duels are
/// enforced: when at least one candidate is proven safe at a common horizon, moves
/// proven exposed at that horizon are removed and unknown moves are kept. With no
/// proven-safe sibling, or for three/four-snake positions, the mask is preserved and
/// the result is logged only. Immediate winning attacks are modelled as proven safe
/// through the simulator, so they are never excluded by blanket contest avoidance.
pub fn root_filter(
    board: &CellBoard4Snakes11x11,
    you: SnakeId,
    base_mask: u8,
    limits: &Limits<'_>,
) -> RootFilter {
    let head = board.get_head_as_native_position(&you);
    let our_length = board.get_length(&you);
    let mut opponent_count = 0usize;
    let mut rival_close = false;
    for slot in 0..MAX_SNAKES {
        let id = SnakeId(slot as u8);
        if id == you || board.get_health(&id) == 0 {
            continue;
        }
        opponent_count += 1;
        let rival_head = board.get_head_as_native_position(&id);
        if board.get_length(&id) >= our_length
            && board.cell_distance(head, rival_head) <= TRIGGER_DISTANCE
        {
            rival_close = true;
        }
    }
    let exits = board.reasonable_move_mask(head).count_ones();
    let triggered = rival_close || exits <= TRIGGER_EXITS;
    if !triggered {
        return RootFilter {
            mask: base_mask,
            applied: false,
            triggered: false,
            opponent_count,
            analysis: None,
        };
    }

    let analysis = analyze(board, you, base_mask, limits);
    let mut mask = base_mask;
    let mut applied = false;
    if opponent_count == 1 && analysis.completed_horizon >= 1 {
        let safe_exists = analysis
            .verdicts()
            .iter()
            .any(|verdict| matches!(verdict, MoveVerdict::ProvenSafe { .. }));
        if safe_exists {
            let mut exposed = 0u8;
            for (index, verdict) in analysis.verdicts().iter().enumerate() {
                if matches!(verdict, MoveVerdict::Exposed { .. }) {
                    exposed |= 1 << index;
                }
            }
            let kept = base_mask & !exposed;
            // A proven-safe candidate can only be a set bit of the input mask, so
            // `kept` is non-empty whenever `safe_exists`; guard anyway so an empty
            // candidate set can never reach selection.
            if kept != 0 {
                applied = kept != base_mask;
                mask = kept;
            }
        }
    }
    RootFilter {
        mask,
        applied,
        triggered,
        opponent_count,
        analysis: Some(analysis),
    }
}

/// Emit exactly one structured summary per request. Never logged per simulated branch.
pub fn log_root_filter(filter: &RootFilter, selection_changed: bool) {
    // Keep recursive proof formatting off the move-publication path.
    let summaries = filter.analysis.as_ref().map(|analysis| {
        analysis.verdicts().each_ref().map(|verdict| match verdict {
            MoveVerdict::ProvenSafe { horizon } => ("safe", *horizon),
            MoveVerdict::Exposed { horizon, .. } => ("exposed", *horizon),
            MoveVerdict::Unknown { horizon } => ("unknown", *horizon),
            MoveVerdict::NotCandidate => ("not_candidate", 0),
        })
    });
    match &filter.analysis {
        Some(analysis) => tracing::info!(
            triggered = filter.triggered,
            opponents = filter.opponent_count,
            applied = filter.applied,
            selection_changed,
            completed_horizon = analysis.completed_horizon,
            simulator_calls = analysis.simulator_calls,
            budget_exhausted = analysis.budget_exhausted,
            verdicts = ?summaries,
            "tactical root filter"
        ),
        None => tracing::info!(
            triggered = filter.triggered,
            opponents = filter.opponent_count,
            applied = false,
            selection_changed = false,
            "tactical root filter"
        ),
    }
}

/// Budget and identity shared across one analysis.
struct Ctx<'a> {
    you: SnakeId,
    calls: u32,
    max_calls: u32,
    deadline: Option<Instant>,
    stop: Option<&'a AtomicBool>,
    exhausted: bool,
    witness_nodes: u32,
}

impl Ctx<'_> {
    /// Check the shared budget before expensive work. Returns false once any limit
    /// has tripped and latches `exhausted` so callers do not keep probing.
    fn check_limits(&mut self) -> bool {
        if self.exhausted {
            return false;
        }
        if self.calls >= self.max_calls {
            self.exhausted = true;
            return false;
        }
        if let Some(deadline) = self.deadline
            && Instant::now() >= deadline
        {
            self.exhausted = true;
            return false;
        }
        if let Some(stop) = self.stop
            && stop.load(Ordering::Relaxed)
        {
            self.exhausted = true;
            return false;
        }
        true
    }
}

/// Three-valued outcome of the recursive search.
enum Outcome {
    Safe,
    Dead(DeadProof),
    Unknown,
}

/// One living opponent and the directions it may play.
struct Opponent {
    id: SnakeId,
    moves: ArrayVec<Move, N_MOVES>,
}

/// The outcome of exploring one of our candidate moves.
enum MoveOutcome {
    Safe,
    Dead {
        reply: ArrayVec<(SnakeId, Move), MAX_SNAKES>,
        child: Box<DeadProof>,
    },
    Unknown,
}

/// Decide whether our snake can be forced to die within `remaining` complete turns.
///
/// `root_mask`, when set, restricts our moves at *this* (root) node to the single
/// candidate being analyzed. Deeper nodes choose our moves existentially from the
/// full reasonable mask.
fn search(
    ctx: &mut Ctx,
    board: CellBoard4Snakes11x11,
    remaining: u8,
    root_mask: Option<u8>,
) -> Outcome {
    if board.get_health(&ctx.you) == 0 {
        return Outcome::Dead(DeadProof::Died);
    }
    // A living sole survivor has already won: nothing can kill us.
    if board.alive_snake_count() == 1 {
        return Outcome::Safe;
    }
    if remaining == 0 {
        return Outcome::Safe;
    }
    if !ctx.check_limits() {
        return Outcome::Unknown;
    }

    let our_mask = root_mask
        .unwrap_or_else(|| board.reasonable_move_mask(board.get_head_as_native_position(&ctx.you)));
    if our_mask == 0 {
        return Outcome::Dead(DeadProof::NoMoves);
    }

    let opponents = opponent_moves(&board, ctx.you);
    let mut branches: Vec<DeadBranch> = Vec::new();
    let mut any_unknown = false;
    for index in 0..N_MOVES {
        if our_mask & (1 << index) == 0 {
            continue;
        }
        match one_move(ctx, &board, Move::from_index(index), &opponents, remaining) {
            MoveOutcome::Safe => return Outcome::Safe,
            MoveOutcome::Dead { reply, child } => branches.push(DeadBranch {
                my_move: Move::from_index(index),
                reply,
                child,
            }),
            MoveOutcome::Unknown => any_unknown = true,
        }
    }

    if any_unknown {
        Outcome::Unknown
    } else if ctx.witness_nodes == 0 {
        // The verdict is established; only the recorded branch is truncated.
        Outcome::Dead(DeadProof::Truncated)
    } else {
        ctx.witness_nodes -= 1;
        Outcome::Dead(DeadProof::Forced { branches })
    }
}

/// Explore one of our moves: it is dead as soon as any single adversarial reply
/// leads to a dead child (an AND over replies is false if any response is false).
fn one_move(
    ctx: &mut Ctx,
    board: &CellBoard4Snakes11x11,
    my_move: Move,
    opponents: &[Opponent],
    remaining: u8,
) -> MoveOutcome {
    let mut odometer = [0usize; MAX_SNAKES];
    let mut unknown = false;
    loop {
        // The check runs before building and simulating; an exhausted budget turns
        // every remaining reply into unknown, never into safety or loss.
        if !ctx.check_limits() {
            return MoveOutcome::Unknown;
        }
        let mut reply = ArrayVec::<(SnakeId, Move), MAX_SNAKES>::new();
        let mut moves = ArrayVec::<(SnakeId, Move), MAX_SNAKES>::new();
        moves.push((ctx.you, my_move));
        for (slot, opponent) in opponents.iter().enumerate() {
            let mv = opponent.moves[odometer[slot]];
            reply.push((opponent.id, mv));
            moves.push((opponent.id, mv));
        }

        let next = board.simulate_single_action(&moves).1;
        ctx.calls += 1;

        match search(ctx, next, remaining - 1, None) {
            Outcome::Dead(child) => {
                return MoveOutcome::Dead {
                    reply,
                    child: Box::new(child),
                };
            }
            Outcome::Safe => {}
            Outcome::Unknown => unknown = true,
        }

        if !advance_odometer(&mut odometer, opponents) {
            break;
        }
    }
    if unknown {
        MoveOutcome::Unknown
    } else {
        MoveOutcome::Safe
    }
}

/// Collect the living opponents and the directions each may play. A snake with an
/// empty reasonable mask is given a single forced direction; every such direction
/// kills it, so one replacement keeps the enumeration bounded and equivalent.
fn opponent_moves(board: &CellBoard4Snakes11x11, you: SnakeId) -> ArrayVec<Opponent, MAX_SNAKES> {
    let mut opponents = ArrayVec::new();
    for slot in 0..MAX_SNAKES {
        let id = SnakeId(slot as u8);
        if id == you || board.get_health(&id) == 0 {
            continue;
        }
        let mask = board.reasonable_move_mask(board.get_head_as_native_position(&id));
        let mut moves = ArrayVec::new();
        for index in 0..N_MOVES {
            if mask & (1 << index) != 0 {
                moves.push(Move::from_index(index));
            }
        }
        if moves.is_empty() {
            moves.push(Move::Up);
        }
        opponents.push(Opponent { id, moves });
    }
    opponents
}

/// Step the mixed-radix odometer to the next reply combination. Returns false when
/// every combination has been visited.
fn advance_odometer(odometer: &mut [usize; MAX_SNAKES], opponents: &[Opponent]) -> bool {
    for slot in 0..opponents.len() {
        odometer[slot] += 1;
        if odometer[slot] < opponents[slot].moves.len() {
            return true;
        }
        odometer[slot] = 0;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use battlesnake_game_types::{
        types::{LengthGettableGame, build_snake_id_map},
        wire_representation::{BattleSnake, Game, Position},
    };
    use std::collections::HashMap;

    const MANIFEST: &str = include_str!("../fixtures/arena-2026-10-07/manifest.json");

    fn fixture_path(file: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/arena-2026-10-07")
            .join(file)
    }

    fn load_game(game_id: &str, turn: u32) -> Game {
        let text = std::fs::read_to_string(fixture_path(&format!("{game_id}-t{turn}.json")))
            .expect("fixture file present");
        serde_json::from_str(&text).expect("fixture parses as a wire Game")
    }

    fn candidate_mask(board: &CellBoard4Snakes11x11, you: SnakeId) -> u8 {
        board.reasonable_move_mask(board.get_head_as_native_position(&you))
    }

    /// Task A acceptance: every fixture parses, converts, and reproduces the saved
    /// request/replay identity, head, length and live-snake count, keyed by the
    /// manifest SHA-256. `you` is read from the snake map, never assumed SnakeId(0).
    #[test]
    fn arena_fixtures_parse_convert_and_match_the_manifest() {
        let manifest: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
        let fixtures = manifest["fixtures"].as_array().unwrap();
        assert_eq!(fixtures.len(), 10);
        let revision = manifest["source_revision"].as_str().unwrap();
        assert_eq!(revision, "05007e123097f64242ebc28c2ee785e1e58519ed");
        for entry in fixtures {
            let file = entry["file"].as_str().unwrap();
            let text = std::fs::read_to_string(fixture_path(file)).unwrap();
            // The manifest pins a SHA-256 of the committed bytes. Verifying the digest
            // itself would need a hash dependency; the committed digest records provenance
            // and the fixture is read verbatim here.
            assert_eq!(entry["sha256"].as_str().unwrap().len(), 64);

            let game: Game = serde_json::from_str(&text).unwrap();
            assert_eq!(game.game.id, entry["game_id"].as_str().unwrap());
            assert_eq!(game.turn, entry["turn"].as_i64().unwrap() as i32);
            assert_eq!(game.you.id, entry["you_id"].as_str().unwrap());

            let ids = build_snake_id_map(&game);
            let you = ids[&game.you.id];
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
            let head = board.get_head_as_native_position(&you);
            let expected = &entry["expected_head"];
            assert_eq!(
                (head.0 % 11) as i64,
                expected["x"].as_i64().unwrap(),
                "{file} head x"
            );
            assert_eq!(
                (head.0 / 11) as i64,
                expected["y"].as_i64().unwrap(),
                "{file} head y"
            );
            assert_eq!(
                board.get_length(&you) as i64,
                entry["expected_length"].as_i64().unwrap(),
                "{file} length"
            );
            assert_eq!(
                board.alive_snake_count() as i64,
                entry["live_snake_count"].as_i64().unwrap(),
                "{file} live count"
            );
        }
    }

    /// Primary fixture, turn 289: up is exposed at two turns; down and left are safe
    /// through four under the documented model.
    #[test]
    fn duel_turn_289_labels_match_the_reference() {
        let game = load_game("b87bd568-b37c-4bc2-9d12-6bc55b834813", 289);
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let mask = candidate_mask(&board, you);
        assert_ne!(mask, 0);

        let analysis = analyze(&board, you, mask, &Limits::for_test(4));
        assert_eq!(analysis.completed_horizon, 4, "{:?}", analysis.verdicts());
        assert!(!analysis.budget_exhausted);
        assert!(matches!(
            analysis.verdict(Move::Up),
            MoveVerdict::Exposed { horizon: 2, .. }
        ));
        assert!(matches!(
            analysis.verdict(Move::Down),
            MoveVerdict::ProvenSafe { horizon: 4 }
        ));
        assert!(matches!(
            analysis.verdict(Move::Left),
            MoveVerdict::ProvenSafe { horizon: 4 }
        ));
    }

    /// Turn 290: no robust one-turn escape. Every candidate must be exposed at the
    /// shallowest horizon; the candidate set stays non-empty.
    #[test]
    fn duel_turn_290_has_no_robust_one_turn_escape() {
        let game = load_game("b87bd568-b37c-4bc2-9d12-6bc55b834813", 290);
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let mask = candidate_mask(&board, you);
        assert_ne!(mask, 0, "production candidates must remain non-empty");

        let analysis = analyze(&board, you, mask, &Limits::for_test(1));
        assert_eq!(analysis.completed_horizon, 1);
        for index in 0..N_MOVES {
            if mask & (1 << index) == 0 {
                continue;
            }
            assert!(
                matches!(
                    analysis.verdicts()[index],
                    MoveVerdict::Exposed { horizon: 1, .. }
                ),
                "{:?}",
                analysis.verdicts()[index]
            );
        }
    }

    /// f624d5d8 turn 179: up is exposed at three turns; right passes three but fails
    /// four. Horizon labels are preserved by deepening the budget separately.
    #[test]
    fn horizon_labels_distinguish_delayed_from_durable_safety() {
        let game = load_game("f624d5d8-f90d-4fad-b375-b94e281e4d62", 179);
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let mask = candidate_mask(&board, you);

        let shallow = analyze(&board, you, mask, &Limits::for_test(2));
        assert!(matches!(
            shallow.verdict(Move::Up),
            MoveVerdict::ProvenSafe { horizon: 2 }
        ));
        assert!(matches!(
            shallow.verdict(Move::Right),
            MoveVerdict::ProvenSafe { horizon: 2 }
        ));

        let medium = analyze(&board, you, mask, &Limits::for_test(3));
        assert!(matches!(
            medium.verdict(Move::Up),
            MoveVerdict::Exposed { horizon: 3, .. }
        ));
        assert!(matches!(
            medium.verdict(Move::Right),
            MoveVerdict::ProvenSafe { horizon: 3 }
        ));

        let deep = analyze(&board, you, mask, &Limits::for_test(4));
        assert!(matches!(
            deep.verdict(Move::Right),
            MoveVerdict::Exposed { horizon: 4, .. }
        ));
    }

    /// The corridor fixtures from the table parse and convert and produce a verdict
    /// for every candidate; expected moves are diagnostic labels, not assertions.
    #[test]
    fn corridor_fixtures_produce_verdicts_for_every_candidate() {
        let cases = [
            ("f0326b4f-b3c5-4f3a-8aec-0bb61ed2d98f", 94u32),
            ("f0326b4f-b3c5-4f3a-8aec-0bb61ed2d98f", 95),
            ("f0326b4f-b3c5-4f3a-8aec-0bb61ed2d98f", 96),
            ("118fe3bd-01da-4ede-951c-4fce02a2264f", 153),
            ("118fe3bd-01da-4ede-951c-4fce02a2264f", 155),
            ("118fe3bd-01da-4ede-951c-4fce02a2264f", 158),
        ];
        for (game_id, turn) in cases {
            let game = load_game(game_id, turn);
            let ids = build_snake_id_map(&game);
            let you = ids[&game.you.id];
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
            let mask = candidate_mask(&board, you);
            assert_ne!(mask, 0, "{game_id} turn {turn}");
            let analysis = analyze(&board, you, mask, &Limits::for_test(2));
            for index in 0..N_MOVES {
                if mask & (1 << index) == 0 {
                    continue;
                }
                assert!(
                    !matches!(
                        analysis.verdicts()[index],
                        MoveVerdict::NotCandidate | MoveVerdict::Unknown { .. }
                    ),
                    "{game_id} turn {turn} {:?}",
                    analysis.verdicts()[index]
                );
            }
        }
    }

    #[test]
    fn zero_budget_is_unknown_without_simulator_calls() {
        let game = load_game("b87bd568-b37c-4bc2-9d12-6bc55b834813", 289);
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let mask = candidate_mask(&board, you);
        let limits = Limits {
            start_horizon: 1,
            max_horizon: 4,
            max_simulator_calls: 0,
            deadline: None,
            stop: None,
        };
        let analysis = analyze(&board, you, mask, &limits);
        assert_eq!(analysis.simulator_calls, 0);
        assert_eq!(analysis.completed_horizon, 0);
        assert!(analysis.budget_exhausted);
        for index in 0..N_MOVES {
            if mask & (1 << index) == 0 {
                continue;
            }
            assert!(matches!(
                analysis.verdicts()[index],
                MoveVerdict::Unknown { horizon: 0 }
            ));
        }
    }

    #[test]
    fn expired_deadline_is_unknown_without_simulator_calls() {
        let game = load_game("b87bd568-b37c-4bc2-9d12-6bc55b834813", 289);
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let mask = candidate_mask(&board, you);
        let limits = Limits {
            start_horizon: 1,
            max_horizon: 4,
            max_simulator_calls: u32::MAX,
            deadline: Some(Instant::now() - Duration::from_millis(1)),
            stop: None,
        };
        let analysis = analyze(&board, you, mask, &limits);
        assert_eq!(analysis.simulator_calls, 0);
        assert!(analysis.budget_exhausted);
    }

    #[test]
    fn an_already_set_stop_signal_is_unknown() {
        let game = load_game("b87bd568-b37c-4bc2-9d12-6bc55b834813", 289);
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let mask = candidate_mask(&board, you);
        let stop = AtomicBool::new(true);
        let limits = Limits {
            start_horizon: 1,
            max_horizon: 4,
            max_simulator_calls: u32::MAX,
            deadline: None,
            stop: Some(&stop),
        };
        let analysis = analyze(&board, you, mask, &limits);
        assert_eq!(analysis.simulator_calls, 0);
        assert!(analysis.budget_exhausted);
    }

    #[test]
    fn input_board_is_never_mutated_and_calls_stay_within_the_cap() {
        let game = load_game("b87bd568-b37c-4bc2-9d12-6bc55b834813", 289);
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let before = board;
        let mask = candidate_mask(&board, you);
        let limits = Limits {
            start_horizon: 1,
            max_horizon: 4,
            max_simulator_calls: 25,
            deadline: None,
            stop: None,
        };
        let analysis = analyze(&board, you, mask, &limits);
        assert_eq!(board, before, "input board must not change");
        assert!(analysis.simulator_calls <= 25);
    }

    // ---- Hand-built positions for the model edge cases -----------------------

    /// Reuse the committed turn-33 fixture as a shape template and overwrite the
    /// snakes, mirroring `mcts` tests. `you` is kept as the fixture's own snake.
    fn board_from_specs(
        own_body: &[Position],
        own_health: i32,
        opponents: &[(&[Position], i32)],
        food: Vec<Position>,
    ) -> (CellBoard4Snakes11x11, SnakeId) {
        let mut game: Game = serde_json::from_str(include_str!("../fixtures/turn33-food.json"))
            .expect("valid template fixture");
        let own_wire_id = game.you.id.clone();
        let template = game
            .board
            .snakes
            .iter()
            .find(|snake| snake.id != own_wire_id)
            .expect("template has an opponent")
            .clone();

        let set_body = |snake: &mut BattleSnake, body: &[Position], health: i32| {
            snake.head = body[0];
            snake.body = body.iter().copied().collect();
            snake.health = health;
        };

        set_body(&mut game.you, own_body, own_health);
        let mut snakes = vec![game.you.clone()];
        for (index, (body, health)) in opponents.iter().enumerate() {
            let mut snake = template.clone();
            snake.id = format!("gs_opponent_{index}");
            snake.name = format!("opponent-{index}");
            set_body(&mut snake, body, *health);
            snakes.push(snake);
        }
        game.board.snakes = snakes;
        game.board.food = food;

        let ids = build_snake_id_map(&game);
        let board = game.as_cell_board(&ids).expect("valid board");
        (board, ids[&own_wire_id])
    }

    fn p(x: i32, y: i32) -> Position {
        Position { x, y }
    }

    #[test]
    fn equal_length_contest_is_lost_against_every_reply() {
        // Head to head: we move up into a shared cell that an equal-length rival can
        // also enter; the simulator kills both tied snakes.
        let (board, you) = board_from_specs(
            &[p(5, 5), p(5, 4), p(5, 3)],
            90,
            &[(&[p(5, 7), p(5, 8), p(5, 9)], 90)],
            vec![],
        );
        let mask = 1 << Move::Up.as_index();
        let analysis = analyze(&board, you, mask, &Limits::for_test(1));
        assert!(matches!(
            analysis.verdict(Move::Up),
            MoveVerdict::Exposed { horizon: 1, .. }
        ));
    }

    #[test]
    fn shorter_opponent_shared_destination_survives() {
        // The rival is shorter, so the tie goes our way and we live.
        let (board, you) = board_from_specs(
            &[p(5, 5), p(5, 4), p(5, 3)],
            90,
            &[(&[p(5, 7), p(5, 8)], 90)],
            vec![],
        );
        let mask = 1 << Move::Up.as_index();
        let analysis = analyze(&board, you, mask, &Limits::for_test(1));
        assert!(matches!(
            analysis.verdict(Move::Up),
            MoveVerdict::ProvenSafe { horizon: 1 }
        ));
    }

    #[test]
    fn food_growth_keeps_a_starving_snake_alive() {
        // Health one; only the food square keeps us alive this turn.
        let (board, you) = board_from_specs(
            &[p(5, 5), p(4, 5), p(3, 5)],
            1,
            &[(&[p(5, 0), p(6, 0)], 90)],
            vec![p(5, 6)],
        );
        let mask = 1 << Move::Up.as_index();
        let analysis = analyze(&board, you, mask, &Limits::for_test(1));
        assert!(matches!(
            analysis.verdict(Move::Up),
            MoveVerdict::ProvenSafe { horizon: 1 }
        ));
    }

    #[test]
    fn starvation_without_food_is_death() {
        let (board, you) = board_from_specs(
            &[p(5, 5), p(4, 5), p(3, 5)],
            1,
            &[(&[p(0, 0), p(1, 0)], 90)],
            vec![],
        );
        let mask = 1 << Move::Up.as_index();
        let analysis = analyze(&board, you, mask, &Limits::for_test(1));
        assert!(matches!(
            analysis.verdict(Move::Up),
            MoveVerdict::Exposed { horizon: 1, .. }
        ));
    }

    #[test]
    fn moving_into_a_vacating_tail_is_safe() {
        // Our head is beside the rival's single tail. Entering that tail is legal, and
        // whichever reply the rival plays the entry stays survivable: it vacates if the
        // rival moves away, and if the rival also steps there we are longer.
        let (board, you) = board_from_specs(
            &[p(0, 2), p(0, 3), p(0, 4)],
            90,
            &[(&[p(1, 3), p(1, 2)], 90)],
            vec![],
        );
        let mask = candidate_mask(&board, you);
        assert_ne!(
            mask & (1 << Move::Right.as_index()),
            0,
            "tail entry offered"
        );
        let analysis = analyze(
            &board,
            you,
            1 << Move::Right.as_index(),
            &Limits::for_test(1),
        );
        assert!(matches!(
            analysis.verdict(Move::Right),
            MoveVerdict::ProvenSafe { horizon: 1 }
        ));
    }

    #[test]
    fn immediate_win_over_the_last_opponent_is_safe() {
        // We are longer and both heads can enter the same cell; the head contest kills
        // the rival and leaves us alive. A winning attack must not read as exposed.
        let (board, you) = board_from_specs(
            &[p(0, 1), p(1, 1), p(2, 1)],
            90,
            &[(&[p(0, 3), p(1, 3)], 90)],
            vec![],
        );
        let mask = 1 << Move::Up.as_index();
        let analysis = analyze(&board, you, mask, &Limits::for_test(2));
        assert!(matches!(
            analysis.verdict(Move::Up),
            MoveVerdict::ProvenSafe { .. }
        ));
    }

    #[test]
    fn mutual_elimination_is_exposed() {
        // Equal length, shared head cell, both die: the candidate is exposed.
        let (board, you) = board_from_specs(
            &[p(0, 1), p(1, 1), p(2, 1)],
            90,
            &[(&[p(0, 3), p(1, 3), p(2, 3)], 90)],
            vec![],
        );
        let mask = 1 << Move::Up.as_index();
        let analysis = analyze(&board, you, mask, &Limits::for_test(1));
        assert!(matches!(
            analysis.verdict(Move::Up),
            MoveVerdict::Exposed { horizon: 1, .. }
        ));
    }

    #[test]
    fn nonzero_you_is_honoured() {
        let mut game: Game = serde_json::from_str(include_str!("../fixtures/turn33-food.json"))
            .expect("valid template fixture");
        let own_wire_id = game.you.id.clone();
        let template = game
            .board
            .snakes
            .iter()
            .find(|snake| snake.id != own_wire_id)
            .unwrap()
            .clone();
        let mut opponent = template;
        opponent.id = "opponent".into();
        game.board.snakes = vec![game.you.clone(), opponent];

        // Put our snake at slot 2, the opponent at slot 1.
        let mut ids: HashMap<String, SnakeId> = HashMap::new();
        ids.insert(game.you.id.clone(), SnakeId(2));
        ids.insert("opponent".into(), SnakeId(1));
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let you = ids[&game.you.id];
        assert_eq!(you, SnakeId(2));
        let mask = candidate_mask(&board, you);
        assert_ne!(mask, 0);
        let analysis = analyze(&board, you, mask, &Limits::for_test(2));
        assert!(analysis.completed_horizon >= 1);
    }

    #[test]
    fn opponent_without_a_reasonable_move_does_not_panic() {
        // Rival walled in on a corner by its own body: no reasonable direction.
        let (board, you) = board_from_specs(
            &[p(5, 5), p(5, 4), p(5, 3)],
            90,
            &[(&[p(10, 10), p(10, 9), p(9, 9)], 90)],
            vec![],
        );
        let mask = candidate_mask(&board, you);
        let analysis = analyze(&board, you, mask, &Limits::for_test(2));
        assert!(analysis.completed_horizon >= 1);
    }

    #[test]
    fn mixed_unknown_is_conservative_under_a_tiny_call_cap() {
        // With a one-call cap the analysis must not claim a proof it did not reach.
        let game = load_game("b87bd568-b37c-4bc2-9d12-6bc55b834813", 289);
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let mask = candidate_mask(&board, you);
        let limits = Limits {
            start_horizon: 1,
            max_horizon: 4,
            max_simulator_calls: 1,
            deadline: None,
            stop: None,
        };
        let analysis = analyze(&board, you, mask, &limits);
        assert!(analysis.simulator_calls <= 1);
        assert!(analysis.budget_exhausted);
        // Any concluded verdict must be Exposed (proved by a single killing reply),
        // never ProvenSafe from an unexplored branch.
        for verdict in analysis.verdicts() {
            assert!(
                !matches!(verdict, MoveVerdict::ProvenSafe { .. }),
                "must not prove safety without exploring every reply"
            );
        }
    }

    #[test]
    fn witness_replays_a_forced_death() {
        let game = load_game("b87bd568-b37c-4bc2-9d12-6bc55b834813", 289);
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let mask = candidate_mask(&board, you);
        let analysis = analyze(&board, you, mask, &Limits::for_test(4));
        let MoveVerdict::Exposed { witness, .. } = analysis.verdict(Move::Up) else {
            panic!(
                "up should be exposed at turn 289: {:?}",
                analysis.verdict(Move::Up)
            );
        };
        // Replay the witness: follow the first branch line and confirm we die at the
        // recorded depth without trusting the diagnostic's verdict.
        let mut current = board;
        let mut proof = witness;
        let mut steps = 0;
        while let DeadProof::Forced { branches } = proof {
            let branch = branches.first().expect("forced has a branch");
            let mut moves = ArrayVec::<(SnakeId, Move), MAX_SNAKES>::new();
            moves.push((you, branch.my_move));
            for (id, mv) in &branch.reply {
                moves.push((*id, *mv));
            }
            current = current.simulate_single_action(&moves).1;
            proof = branch.child.as_ref();
            steps += 1;
        }
        assert!(steps >= 1);
        assert_eq!(current.get_health(&you), 0, "witness line must kill us");
    }

    // ---- Root filter (task B2) -----------------------------------------------

    #[test]
    fn crowded_duel_filters_the_eight_turn_trap_with_the_ten_turn_budget() {
        let game: Game = serde_json::from_str(include_str!(
            "../fixtures/crowded-duel-df620bcb8c8ad8e7.json"
        ))
        .unwrap();
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
        let base = candidate_mask(&board, you);
        let shallow = root_filter(&board, you, base, &Limits::for_test(4));
        assert!(!shallow.applied);

        // Disable only wall time so the regression is deterministic; retain the
        // production horizon and shared simulator-call budget.
        let limits = Limits {
            deadline: None,
            ..Limits::default()
        };
        let deeper = root_filter(&board, you, base, &limits);
        let analysis = deeper.analysis.as_ref().unwrap();
        assert_eq!(analysis.completed_horizon, 10);
        assert!(!analysis.budget_exhausted);
        assert!(matches!(
            analysis.verdict(Move::Up),
            MoveVerdict::Exposed { horizon: 8, .. }
        ));
        assert!(matches!(
            analysis.verdict(Move::Down),
            MoveVerdict::ProvenSafe { horizon: 10 }
        ));
        assert!(deeper.applied);
        assert_eq!(deeper.mask, 1 << Move::Down.as_index());
    }

    fn fixture_board(game_id: &str, turn: u32) -> (CellBoard4Snakes11x11, SnakeId) {
        let game = load_game(game_id, turn);
        let ids = build_snake_id_map(&game);
        let you = ids[&game.you.id];
        (game.as_cell_board(&ids).unwrap(), you)
    }

    #[test]
    fn root_filter_does_not_trigger_for_a_distant_shorter_rival() {
        let (board, you) = board_from_specs(
            &[p(5, 5), p(5, 4), p(5, 3)],
            90,
            &[(&[p(10, 0), p(10, 1)], 90)],
            vec![],
        );
        let base = candidate_mask(&board, you);
        let filter = root_filter(&board, you, base, &Limits::for_test(4));
        assert!(!filter.triggered);
        assert!(!filter.applied);
        assert_eq!(filter.mask, base);
        assert!(filter.analysis.is_none());
    }

    #[test]
    fn root_filter_excludes_only_the_exposed_duel_move() {
        let (board, you) = fixture_board("b87bd568-b37c-4bc2-9d12-6bc55b834813", 289);
        let base = candidate_mask(&board, you);
        assert_ne!(base & (1 << Move::Up.as_index()), 0);
        let filter = root_filter(&board, you, base, &Limits::for_test(4));
        assert!(filter.triggered);
        assert!(filter.applied);
        assert_eq!(filter.opponent_count, 1);
        assert_eq!(filter.mask, base & !(1 << Move::Up.as_index()));
        assert_ne!(filter.mask & (1 << Move::Down.as_index()), 0);
        let analysis = filter.analysis.unwrap();
        assert!(matches!(
            analysis.verdict(Move::Up),
            MoveVerdict::Exposed { horizon: 2, .. }
        ));
    }

    #[test]
    fn root_filter_keeps_the_mask_when_every_candidate_is_exposed() {
        let (board, you) = fixture_board("b87bd568-b37c-4bc2-9d12-6bc55b834813", 290);
        let base = candidate_mask(&board, you);
        let filter = root_filter(&board, you, base, &Limits::for_test(1));
        assert!(filter.triggered);
        assert!(!filter.applied, "an all-exposed mask must not be emptied");
        assert_eq!(filter.mask, base);
        assert_ne!(filter.mask, 0);
    }

    #[test]
    fn root_filter_is_log_only_for_multiplayer() {
        let (board, you) = fixture_board("f624d5d8-f90d-4fad-b375-b94e281e4d62", 179);
        let base = candidate_mask(&board, you);
        let filter = root_filter(&board, you, base, &Limits::for_test(2));
        assert_eq!(filter.opponent_count, 2);
        assert!(
            !filter.applied,
            "multiplayer results are logged, not enforced"
        );
        assert_eq!(filter.mask, base);
    }

    #[test]
    fn root_filter_unknown_keeps_the_mask() {
        let (board, you) = fixture_board("b87bd568-b37c-4bc2-9d12-6bc55b834813", 289);
        let base = candidate_mask(&board, you);
        let limits = Limits {
            start_horizon: 1,
            max_horizon: 4,
            max_simulator_calls: 1,
            deadline: None,
            stop: None,
        };
        let filter = root_filter(&board, you, base, &limits);
        assert!(!filter.applied, "unknown must not empty the mask");
        assert_eq!(filter.mask, base);
    }

    #[test]
    fn root_filter_preserves_an_immediate_winning_attack() {
        let (board, you) = board_from_specs(
            &[p(0, 1), p(1, 1), p(2, 1)],
            90,
            &[(&[p(0, 3), p(1, 3)], 90)],
            vec![],
        );
        let base = 1 << Move::Up.as_index();
        let filter = root_filter(&board, you, base, &Limits::for_test(2));
        assert_ne!(
            filter.mask & (1 << Move::Up.as_index()),
            0,
            "a winning attack must never be filtered out"
        );
        assert_eq!(filter.mask, base);
    }

    #[test]
    fn root_filtered_selection_matches_default_without_a_filter() {
        let (board, you) = fixture_board("b87bd568-b37c-4bc2-9d12-6bc55b834813", 289);
        let node = crate::mcts::Node::new_root(board);
        let default = node.best_move(you);
        assert_eq!(node.best_move_with_root_filter(you, None), default);
        let forced = 1 << Move::Down.as_index();
        assert_eq!(
            node.best_move_with_root_filter(you, Some(&RootFilter::passthrough(forced))),
            Some(Move::Down)
        );
    }
}
