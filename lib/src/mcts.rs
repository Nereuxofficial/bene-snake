use std::{
    collections::BTreeMap,
    sync::{
        Arc, Condvar, Mutex, OnceLock, RwLock,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
};

use arrayvec::ArrayVec;
use battlesnake_game_types::{
    compact_representation::CellIndex,
    compact_representation::standard::{CellBoard4Snakes11x11, MoveTarget},
    types::{
        Action, FoodGettableGame, FoodQueryableGame, HazardQueryableGame, HeadGettableGame,
        HealthGettableGame, LengthGettableGame, Move, N_MOVES, NeckQueryableGame,
        PositionGettableGame, RandomReasonableMovesGame, SnakeId, VictorDeterminableGame,
    },
    wire_representation::Position,
};
use rand::{Rng, RngExt};
use std::time::{Duration, Instant};

use crate::eval::{evaluate_board, evaluate_board_with_food};
use tracing::info;

#[cfg(test)]
use battlesnake_game_types::{
    compact_representation::standard::moves_from_mask, types::MoveArray, types::ReasonableMovesGame,
};

const MAX_ROLLOUT_DEPTH: u32 = 24;
const MAX_TREE_DEPTH: usize = 64;
const TREE_PATH_CAPACITY: usize = 128;
pub const SEARCH_WORKERS: usize = 8;

fn search_pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(SEARCH_WORKERS)
            .thread_name(|i| format!("mcts-{i}"))
            .build()
            .expect("create MCTS worker pool")
    })
}
/// Cell offset of each `Move::as_index()`, in that index order.
const MOVE_OFFSETS: [(i32, i32); N_MOVES] = [(0, 1), (0, -1), (-1, 0), (1, 0)];
/// `Move::as_index()` that steps from a cell onto a neighbor at offset `(dx, dy)`, laid out by
/// `((dx + 1) * 3 + (dy + 1))` over the `-1..=1` range. Only ever read where
/// `|dx| + |dy| == 1`, so the diagonal and self entries are never selected and hold `Up`.
const MOVE_FROM_OFFSET: [usize; 9] = {
    use Move::{Down, Left, Right, Up};
    let up = Up.as_index();
    let down = Down.as_index();
    [
        up,
        Right.as_index(),
        up,
        up,
        up,
        down,
        up,
        Left.as_index(),
        up,
    ]
};
// Terminal rewards and UCB must use the same scale. See experiments/reward-scale/.
const WIN_REWARD: u32 = 1000;
const LEAF_SCORE_HALF_REWARD: u32 = 1000;
// Give food eaten along the sampled search path lasting value at live leaves.
// Terminal wins still outrank every live result.
const FOOD_GAIN_REWARD: u32 = 100;
const UNIFORM_MOVE_PERCENT: u32 = 20;

#[derive(Clone, Copy)]
struct RolloutOptions {
    food_guidance: bool,
    food_gain_reward: u32,
    depth: u32,
}

fn leaf_reward(score: u16) -> u32 {
    let score = u32::from(score);
    // Preserve heuristic ordering without letting a live leaf equal a terminal win.
    (WIN_REWARD * score / (score + LEAF_SCORE_HALF_REWARD)).clamp(1, WIN_REWARD - 1)
}

/// A cached per-snake policy built from the state before any moves are chosen.
struct MovePolicy {
    weights: [u8; 4],
    total_weight: u32,
    /// Bit `Move::as_index()` is set for each reasonable move whose destination can also
    /// be entered by a living opponent that is at least as long after the move. The
    /// simulator kills every snake tied for the longest new head, so an equal or longer
    /// opponent sharing our destination means we lose that head-to-head.
    losing_head_contests: u8,
}

/// Everything one snake's policy needs that does not depend on which snake is asking. A board
/// is fixed for a whole node, so this is read once per board instead of once per policy.
#[derive(Clone, Copy)]
struct SnakeFacts {
    health: i64,
    length: i64,
    head: Position,
    head_index: CellIndex<u8>,
    /// Bit `Move::as_index()` is set for every reasonable move.
    legal_mask: u8,
}

/// Iterate the moves encoded in a mask, in ascending `Move::as_index()` order.
fn mask_bits(mask: u8) -> impl Iterator<Item = Move> {
    (0..N_MOVES)
        .filter(move |index| mask & (1 << index) != 0)
        .map(Move::from_index)
}

/// Collect the per-snake facts for a board, indexed by `SnakeId`. Dead snakes keep zeroed facts
/// and are never consulted: callers iterate the living snakes they were given.
fn snake_facts_from_masks(
    board: &CellBoard4Snakes11x11,
    masks: &ArrayVec<(SnakeId, u8), 4>,
) -> [SnakeFacts; 4] {
    let mut facts = [SnakeFacts {
        health: 0,
        length: 0,
        head: Position { x: 0, y: 0 },
        head_index: CellIndex::from_usize(0),
        legal_mask: 0,
    }; 4];
    for (id, mask) in masks {
        let index = id.as_usize();
        facts[index] = SnakeFacts {
            health: board.get_health_i64(id),
            length: board.get_length_i64(id),
            head: board.get_head_as_position(id),
            head_index: board.get_head_as_native_position(id),
            legal_mask: *mask,
        };
    }
    facts
}

#[cfg(test)]
fn snake_facts(board: &CellBoard4Snakes11x11) -> [SnakeFacts; 4] {
    snake_facts_from_masks(board, &board.reasonable_move_masks())
}

/// Build one cached small-integer policy per snake from the state before any moves are chosen.
/// The score is deliberately simple: all legal moves start with support, then safe mobility and
/// food progress add weight, while likely losing head contests reduce it. The
/// `losing_head_contests` mask is the same tactical check the tree uses to prune our own moves.
fn move_policy(
    board: &CellBoard4Snakes11x11,
    facts: &[SnakeFacts; 4],
    snake: SnakeId,
    legal: u8,
    food: &[Position],
    food_guidance: bool,
) -> MovePolicy {
    #[cfg(feature = "tracy")]
    let _tracy_span = tracy_client::span!("move_policy");
    let SnakeFacts {
        health,
        length,
        head,
        head_index,
        ..
    } = facts[snake.as_usize()];
    let food_is_urgent = health <= 50;
    let mut policy = MovePolicy {
        weights: [0; 4],
        total_weight: 0,
        losing_head_contests: 0,
    };

    // A single pass over the food list finds the nearest food and, for that nearest food only,
    // which directions shorten the distance to it. The previous code scanned the whole food list
    // once for the head and once again for every legal move.
    let mut nearest_food = u32::MAX;
    let mut food_progress_mask = 0u8;
    for pos in food {
        let dx = pos.x - head.x;
        let dy = pos.y - head.y;
        let distance = dx.unsigned_abs() + dy.unsigned_abs();
        if distance > nearest_food {
            continue;
        }
        if distance < nearest_food {
            nearest_food = distance;
            food_progress_mask = 0;
        }
        if dx > 0 {
            food_progress_mask |= 1 << Move::Right.as_index();
        } else if dx < 0 {
            food_progress_mask |= 1 << Move::Left.as_index();
        }
        if dy > 0 {
            food_progress_mask |= 1 << Move::Up.as_index();
        } else if dy < 0 {
            food_progress_mask |= 1 << Move::Down.as_index();
        }
    }
    let makes_food_progress = |mv: &Move| food_progress_mask & (1 << mv.as_index()) != 0;

    // Which of our destinations each qualifying opponent could also enter. The opponent's head
    // is fixed, so the adjacency test runs once here instead of once per move per opponent.
    // Offsets are plain integers: a destination is one step from our own head, so an opponent
    // more than two steps away cannot contest anything.
    let mut contests: [(SnakeId, i64, u8); 4] = [(SnakeId(0), 0, 0); 4];
    let mut contest_count = 0;
    for (index, other) in facts.iter().enumerate() {
        if index == snake.as_usize() || other.health == 0 || other.length < length {
            continue;
        }
        let (ox, oy) = (other.head.x - head.x, other.head.y - head.y);
        if ox.abs() + oy.abs() > 2 {
            continue;
        }
        // The opponent reaches one of our destinations when that destination is exactly one step
        // from its head. With the offsets unrolled, the whole geometry is constant arithmetic and
        // the resulting approach direction is a compile-time index.
        let mut contested = 0u8;
        for (index, &(mx, my)) in MOVE_OFFSETS.iter().enumerate() {
            let (dx, dy) = (ox - mx, oy - my);
            if dx.abs() + dy.abs() != 1 {
                continue;
            }
            // The opponent's head sits one step off our destination, so it approaches from the
            // opposite side of us.
            let approach = MOVE_FROM_OFFSET[(dx + 1) as usize * 3 + (dy + 1) as usize];
            if other.legal_mask & (1 << approach) != 0 {
                contested |= 1 << index;
            }
        }
        if contested != 0 {
            contests[contest_count] = (SnakeId(index as u8), other.health, contested);
            contest_count += 1;
        }
    }

    // Iterating the mask by index rather than through `mask_bits` keeps `1 << index` and
    // `Move::from_index` constant-folded, so the whole move body is straight-line code.
    for index in 0..N_MOVES {
        let bit = 1u8 << index;
        if legal & bit == 0 {
            continue;
        }
        let mv = Move::from_index(index);
        let target = board.describe_move(snake, head_index, mv);
        // The destination must only be inspected after validating bounds: when every
        // direction is blocked the simulator's conventional fallback is Up, even off-board.
        let has_food = target.is_food;
        let immediate_hazard_death =
            target.is_hazard && health <= 1 + i64::from(board.get_hazard_damage()) && !has_food;
        let starvation_death = health <= 1 && !has_food;
        let dies_immediately =
            !target.on_board || immediate_hazard_death || starvation_death || target.is_own_neck;

        let mut weight = 4u8;
        if dies_immediately {
            weight = 1;
        } else {
            // Mobility is only scored for surviving moves, so read it lazily rather than
            // inside `describe_move` where a fatal move would pay for four cell reads.
            weight += board.free_neighbor_count(target.destination) * 2;
            if food_guidance {
                if has_food {
                    weight += if health <= 20 {
                        12
                    } else if food_is_urgent {
                        8
                    } else {
                        4
                    };
                } else if makes_food_progress(&mv) {
                    weight += if health <= 20 {
                        5
                    } else if food_is_urgent {
                        3
                    } else {
                        1
                    };
                }
            } else if food_is_urgent && has_food {
                weight += 8;
            } else if food_is_urgent && makes_food_progress(&mv) {
                weight += 3;
            }
        }

        // The simulator compares lengths after the move, including growth. Two snakes
        // entering the same square both grow together, so comparing the pre-move lengths
        // here is equivalent. Food still matters because `is_feasible_destination` only
        // keeps a low-health opponent alive when the shared square is food.
        let shared_losing_destination = target.on_board
            && contests[..contest_count]
                .iter()
                .any(|(other, health, contested)| {
                    contested & bit != 0 && is_feasible_destination(board, *other, target, *health)
                });
        if shared_losing_destination {
            policy.losing_head_contests |= bit;
            weight = (weight / 4).max(1);
        }
        policy.weights[index] = weight;
        policy.total_weight += u32::from(weight);
    }
    policy
}

/// Keep only the reasonable moves that do not lose a head-to-head. When every move is
/// contested, fall back to the full reasonable set so search retains a nonempty candidate
/// set and can compare those unavoidable risks.
fn pruned_tree_mask(legal: u8, losing_head_contests: u8) -> u8 {
    let kept = legal & !losing_head_contests;
    if kept == 0 { legal } else { kept }
}

#[cfg(test)]
fn move_weights(board: &CellBoard4Snakes11x11, snake: SnakeId, legal: MoveArray) -> [u8; 4] {
    let food = board.get_all_food_as_positions();
    let mask = legal.iter().fold(0u8, |mask, mv| mask | 1 << mv.as_index());
    move_policy(board, &snake_facts(board), snake, mask, &food, true).weights
}

/// Whether `snake` would survive entering `target` on the next turn. The caller guarantees
/// `target` is one of that snake's head neighbors, so only the free-cell rule, the neck, and the
/// health/hazard budget are left to check. Reading those flags off the target avoids rescanning
/// the head's neighbors, which cost four cell reads per candidate challenger.
fn is_feasible_destination(
    board: &CellBoard4Snakes11x11,
    snake: SnakeId,
    target: MoveTarget<u8>,
    health: i64,
) -> bool {
    if !target.on_board || board.is_neck(&snake, &target.destination) {
        return false;
    }
    let hazard_damage = if target.is_hazard {
        board.get_hazard_damage()
    } else {
        0
    };
    board.cell_is_free(target.destination)
        && (target.is_food || health > 1 + i64::from(hazard_damage))
}

/// Sample one move from `mask`, whose set bits are `Move::as_index()` values. Bit order is
/// ascending, so the choice depends only on the mask, never on how the list was built.
fn sample_policy_move(mask: u8, weights: [u8; 4], total_weight: u32, rng: &mut impl Rng) -> Move {
    let count = mask.count_ones();
    if count == 0 {
        return Move::Up;
    }
    // One draw feeds both the uniform gate and the pick, so a sampled move costs one RNG call
    // instead of two. The high bits gate, the low bits index.
    let draw = rng.random::<u32>();
    let (gate, pick) = (draw >> 16, draw & 0xFFFF);

    // A 20% uniform component keeps every reasonable move possible; the rest follows the cached
    // weighted policy. This avoids a softmax and keeps sampling allocation-free.
    let uniform = gate % 100 < UNIFORM_MOVE_PERCENT || total_weight == 0;
    if uniform {
        let mut remaining = mask;
        let mut index = (pick * count) >> 16;
        while index > 0 {
            remaining &= remaining - 1;
            index -= 1;
        }
        return Move::from_index(remaining.trailing_zeros() as usize);
    }

    let mut sample = (pick * total_weight) >> 16;
    let mut remaining = mask;
    while remaining != 0 {
        // Work with the move's index directly: converting to `Move` and back would re-derive the
        // same index just to index the weight array.
        let index = remaining.trailing_zeros() as usize;
        remaining &= remaining - 1;
        let weight = u32::from(weights[index]);
        if sample < weight {
            return Move::from_index(index);
        }
        sample -= weight;
    }
    Move::from_index(mask.trailing_zeros() as usize)
}

fn sample_rollout_moves(
    board: &CellBoard4Snakes11x11,
    food: &[Position],
    rng: &mut impl Rng,
    food_guidance: bool,
) -> ArrayVec<(SnakeId, Move), 4> {
    #[cfg(feature = "tracy")]
    let _tracy_span = tracy_client::span!("sample_rollout_moves");
    if !food_guidance {
        return board.random_reasonable_move_for_each_snake(rng).collect();
    }
    let masks = board.reasonable_move_masks();
    let facts = snake_facts_from_masks(board, &masks);
    masks
        .iter()
        .map(|(snake, mask)| {
            let policy = move_policy(board, &facts, *snake, *mask, food, true);
            (
                *snake,
                sample_policy_move(*mask, policy.weights, policy.total_weight, rng),
            )
        })
        .collect()
}

#[derive(Default)]
pub struct SearchDepthStats {
    pub iterations: u64,
    pub max_tree_depth: usize,
    pub tree_depth_limit_hits: u64,
    pub max_rollout_depth: u32,
    pub rollout_depth_limit_hits: u64,
}

// Separate frequently updated edges to avoid false sharing between workers.
#[repr(align(64))]
#[derive(Default)]
struct MoveStats {
    visits: AtomicU32,
    reward: AtomicU64,
    in_flight: AtomicU32,
}

/// A board state before all snakes choose their next moves.
pub struct Node {
    board: CellBoard4Snakes11x11,
    escape_guard_enabled: bool,
    escape: OnceLock<(SnakeId, crate::escape::EscapeAnalysis)>,
    food_guidance: bool,
    food_gain_reward: u32,
    rollout_depth: u32,
    children: RwLock<BTreeMap<Action<4>, Arc<Node>>>,
    tree_depth: usize,
    visits: AtomicU32,
    own_moves: [MoveStats; 4],
    move_cache: OnceLock<NodeMoveCache>,
}

/// A cached visit needs only its Arc; do not copy its board into the return value.
// Keep transient boards inline: boxing would allocate on uncached rollouts.
#[allow(clippy::large_enum_variant)]
enum ChildResult {
    Existing(Arc<Node>),
    Expanded(Arc<Node>),
    Unstored(CellBoard4Snakes11x11),
}

/// Children reachable after our issued move. The opponent's simultaneous move is not
/// known until the next request, so retain the searched responses and match the full
/// observed board then. Keeping only visited children avoids retaining an empty tree.
pub struct SearchTreeCache {
    candidates: Vec<Arc<Node>>,
    you: SnakeId,
    root_length: u16,
}

impl SearchTreeCache {
    // A visit expands at most one node, so retained visits bound retained nodes.
    const MAX_RETAINED_VISITS: u32 = 20_000;
    const MAX_CANDIDATES: usize = 32;

    pub fn after_move(root: &Arc<Node>, you: SnakeId, chosen: Move) -> Self {
        let mut candidates: Vec<_> = root
            .children
            .read()
            .unwrap()
            .iter()
            .filter(|(action, child)| {
                action.into_inner()[you.as_usize()] == Some(chosen) && child.visits() > 0
            })
            .map(|(_, child)| Arc::clone(child))
            .collect();
        candidates.sort_unstable_by_key(|child| std::cmp::Reverse(child.visits()));
        let mut retained_visits = 0u32;
        candidates.retain(|child| {
            if retained_visits.saturating_add(child.visits()) > Self::MAX_RETAINED_VISITS {
                return false;
            }
            retained_visits += child.visits();
            true
        });
        candidates.truncate(Self::MAX_CANDIDATES);
        Self {
            candidates,
            you,
            root_length: root.board.get_length(&you),
        }
    }

    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    pub fn match_observed(self, board: &CellBoard4Snakes11x11, you: SnakeId) -> Option<Arc<Node>> {
        // Old reward samples use the former root's length as their food-gain baseline.
        if you != self.you || board.get_length(&you) != self.root_length {
            return None;
        }
        self.candidates
            .into_iter()
            .find(|child| child.board == *board)
    }
}

struct NodeMoveCache {
    /// Reasonable-move masks, shared with opponents and rollouts. The tree no longer selects
    /// from this directly; it is the fallback when pruning contests all moves.
    masks: ArrayVec<(SnakeId, u8), 4>,
    policies: [[u8; 4]; 4],
    policy_totals: [u32; 4],
    /// Per-snake tree masks: opponents also avoid losing contests when an alternative exists.
    tree_masks: [u8; 4],
}

impl Node {
    fn rollout_options(&self) -> RolloutOptions {
        RolloutOptions {
            food_guidance: self.food_guidance,
            food_gain_reward: self.food_gain_reward,
            depth: self.rollout_depth,
        }
    }

    pub fn visits(&self) -> u32 {
        self.visits.load(Ordering::Relaxed)
    }

    pub fn new_root(board: CellBoard4Snakes11x11) -> Self {
        Self::new_root_with_food_gain_reward(board, FOOD_GAIN_REWARD)
    }

    /// Override the food-gain reward for controlled search experiments.
    pub fn new_root_with_food_gain_reward(
        board: CellBoard4Snakes11x11,
        food_gain_reward: u32,
    ) -> Self {
        Self::new_root_with_options(board, true, food_gain_reward, MAX_ROLLOUT_DEPTH)
    }

    /// Construct a root with food guidance configurable for controlled search experiments.
    pub fn new_root_with_food_guidance(board: CellBoard4Snakes11x11, food_guidance: bool) -> Self {
        Self::new_root_with_options(board, food_guidance, FOOD_GAIN_REWARD, MAX_ROLLOUT_DEPTH)
    }

    /// Override rollout horizon for controlled search comparisons.
    pub fn new_root_with_rollout_depth(board: CellBoard4Snakes11x11, rollout_depth: u32) -> Self {
        assert!(rollout_depth > 0);
        Self::new_root_with_options(board, true, FOOD_GAIN_REWARD, rollout_depth)
    }

    fn new_root_with_options(
        board: CellBoard4Snakes11x11,
        food_guidance: bool,
        food_gain_reward: u32,
        rollout_depth: u32,
    ) -> Self {
        Self {
            board,
            escape_guard_enabled: true,
            escape: OnceLock::new(),
            food_guidance,
            food_gain_reward,
            rollout_depth,
            children: RwLock::new(BTreeMap::new()),
            tree_depth: MAX_TREE_DEPTH,
            visits: AtomicU32::new(0),
            own_moves: std::array::from_fn(|_| MoveStats::default()),
            move_cache: OnceLock::new(),
        }
    }

    /// Override the tree horizon for controlled comparisons (rollout horizon is unchanged).
    pub fn new_root_with_tree_depth(board: CellBoard4Snakes11x11, tree_depth: usize) -> Self {
        assert!((1..=TREE_PATH_CAPACITY).contains(&tree_depth));
        let mut root = Self::new_root(board);
        root.tree_depth = tree_depth;
        root
    }

    /// Disable only the root escape checks for controlled comparisons.
    pub fn new_root_with_escape_guard(board: CellBoard4Snakes11x11, enabled: bool) -> Self {
        let mut root = Self::new_root(board);
        root.escape_guard_enabled = enabled;
        root
    }

    fn prepare_escape_guard(&self, you: SnakeId) {
        if !self.escape_guard_enabled {
            return;
        }
        self.escape.get_or_init(|| {
            let cache = self.move_cache();
            (
                you,
                crate::escape::analyze(
                    &self.board,
                    you,
                    cache.tree_masks[you.as_usize()],
                    cache.policies[you.as_usize()],
                ),
            )
        });
    }

    fn selection_policy(&self, you: SnakeId) -> [u8; 4] {
        self.escape.get().filter(|(id, _)| *id == you).map_or(
            self.move_cache().policies[you.as_usize()],
            |(_, analysis)| analysis.weights,
        )
    }

    fn move_cache(&self) -> &NodeMoveCache {
        self.move_cache.get_or_init(|| {
            let masks = self.board.reasonable_move_masks();
            let facts = snake_facts_from_masks(&self.board, &masks);
            let food = self.board.get_all_food_as_positions();
            let mut policies = [[0; 4]; 4];
            let mut policy_totals = [0; 4];
            let mut tree_masks = [0u8; 4];
            for (id, mask) in &masks {
                let policy =
                    move_policy(&self.board, &facts, *id, *mask, &food, self.food_guidance);
                policies[id.as_usize()] = policy.weights;
                tree_masks[id.as_usize()] = pruned_tree_mask(*mask, policy.losing_head_contests);
                policy_totals[id.as_usize()] = mask_bits(tree_masks[id.as_usize()])
                    .map(|mv| u32::from(policy.weights[mv.as_index()]))
                    .sum();
            }
            NodeMoveCache {
                masks,
                policies,
                policy_totals,
                tree_masks,
            }
        })
    }

    pub fn get_depth(&self) -> u32 {
        self.children
            .read()
            .unwrap()
            .values()
            .map(|child| child.get_depth() + 1)
            .max()
            .unwrap_or(0)
    }

    /// The full reasonable move set, shared with opponents and rollouts. The tree no
    /// longer selects from this directly; it is the fallback when pruning contests all moves.
    #[cfg(test)]
    fn legal_own_moves(&self, you: SnakeId) -> Option<MoveArray> {
        self.move_cache()
            .masks
            .iter()
            .find(|(id, _)| *id == you)
            .map(|(_, mask)| moves_from_mask(*mask))
    }

    /// The tree candidate set as a list, for tests that assert on move membership.
    #[cfg(test)]
    fn tree_own_moves(&self, you: SnakeId) -> Option<MoveArray> {
        self.tree_own_mask(you).map(moves_from_mask)
    }

    /// Our tree candidate mask: reasonable moves minus losing head-to-head contests.
    /// Returns `None` only when `you` has no reasonable moves (for example, a dead snake).
    fn tree_own_mask(&self, you: SnakeId) -> Option<u8> {
        let cache = self.move_cache();
        cache.masks.iter().any(|(id, _)| *id == you).then(|| {
            self.escape
                .get()
                .filter(|(id, _)| *id == you)
                .map_or(cache.tree_masks[you.as_usize()], |(_, analysis)| {
                    analysis.mask
                })
        })
    }

    fn select_own_move(&self, you: SnakeId, exploration: f64) -> Option<Move> {
        #[cfg(feature = "tracy")]
        let _tracy_span = tracy_client::span!("select_own_move");
        let mask = self.tree_own_mask(you)?;
        let policy = self.selection_policy(you);
        let prior_total: u32 = mask_bits(mask)
            .map(|mv| u32::from(policy[mv.as_index()]))
            .sum();
        let parent_visits = self.visits.load(Ordering::Relaxed) as f64
            + self
                .own_moves
                .iter()
                .map(|stats| stats.in_flight.load(Ordering::Relaxed) as f64)
                .sum::<f64>();
        // `max_by` evaluates both sides of each comparison, so every term that depends only on
        // the parent was being recomputed for each candidate pair. Hoisting them keeps the exact
        // same floating-point values while turning several `ln` and `sqrt` calls per node visit
        // into one each.
        let parent_exploration_log = (parent_visits + 1.0).ln();
        let parent_prior_scale = (parent_visits + 1.0).sqrt();
        let prior_denominator = f64::from(prior_total.max(1));
        mask_bits(mask)
            .map(|mv| {
                let stats = &self.own_moves[mv.as_index()];
                // In-flight samples act as temporary zero rewards (virtual loss).
                let visits = stats.visits.load(Ordering::Relaxed) as f64
                    + stats.in_flight.load(Ordering::Relaxed) as f64;
                let value = if visits == 0.0 {
                    f64::INFINITY
                } else {
                    let mean = stats.reward.load(Ordering::Relaxed) as f64
                        / (visits * f64::from(WIN_REWARD));
                    let prior_bonus = if self.food_guidance {
                        let prior = f64::from(policy[mv.as_index()]) / prior_denominator;
                        exploration * prior * parent_prior_scale / (visits + 1.0)
                    } else {
                        0.0
                    };
                    mean + exploration * (parent_exploration_log / visits).sqrt() + prior_bonus
                };
                (mv, value)
            })
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(mv, _)| mv)
    }

    /// Choose only bene-snake's move. Opponent responses are averaged through visits.
    /// Only uncontested tree candidates are considered.
    pub fn best_move(&self, you: SnakeId) -> Option<Move> {
        self.prepare_escape_guard(you);
        let mask = self.tree_own_mask(you)?;
        let policy = self.selection_policy(you);
        mask_bits(mask).max_by(|left, right| {
            let stats = |mv: &Move| &self.own_moves[mv.as_index()];
            let left_visits = stats(left).visits.load(Ordering::Relaxed);
            let right_visits = stats(right).visits.load(Ordering::Relaxed);
            left_visits.cmp(&right_visits).then_with(|| {
                let mean = |mv: &Move, visits: u32| {
                    stats(mv).reward.load(Ordering::Relaxed) as f64 / visits.max(1) as f64
                };
                mean(left, left_visits)
                    .total_cmp(&mean(right, right_visits))
                    .then_with(|| {
                        if self.food_guidance {
                            policy[left.as_index()].cmp(&policy[right.as_index()])
                        } else {
                            std::cmp::Ordering::Equal
                        }
                    })
            })
        })
    }

    fn sample_joint_action(
        &self,
        you: SnakeId,
        own_move: Move,
        rng: &mut impl Rng,
    ) -> ArrayVec<(SnakeId, Move), 4> {
        #[cfg(feature = "tracy")]
        let _tracy_span = tracy_client::span!("sample_joint_action");
        let cache = self.move_cache();
        cache
            .masks
            .iter()
            .map(|(id, _)| {
                let mv = if *id == you {
                    own_move
                } else {
                    sample_policy_move(
                        cache.tree_masks[id.as_usize()],
                        cache.policies[id.as_usize()],
                        cache.policy_totals[id.as_usize()],
                        rng,
                    )
                };
                (*id, mv)
            })
            .collect()
    }

    fn child_for_action(&self, action: &[(SnakeId, Move)]) -> ChildResult {
        #[cfg(feature = "tracy")]
        let _tracy_span = tracy_client::span!("child_for_action");
        let key = Action::collect_from(action.iter());
        if let Some(child) = self.children.read().unwrap().get(&key) {
            return ChildResult::Existing(Arc::clone(child));
        }

        let next_board = self.board.simulate_single_action(action).1;
        let max_children = 3 + 2 * (self.visits.load(Ordering::Relaxed) as f64).sqrt() as usize;
        let mut children = self.children.write().unwrap();
        // Another worker may have expanded this action while we simulated it.
        if let Some(child) = children.get(&key) {
            return ChildResult::Existing(Arc::clone(child));
        }
        if children.len() >= max_children {
            return ChildResult::Unstored(next_board);
        }

        let mut child = Node::new_root_with_options(
            next_board,
            self.food_guidance,
            self.food_gain_reward,
            self.rollout_depth,
        );
        child.tree_depth = self.tree_depth;
        let child = Arc::new(child);
        children.insert(key, Arc::clone(&child));
        ChildResult::Expanded(child)
    }

    fn record(&self, own_move: Move, result: u32) {
        let stats = &self.own_moves[own_move.as_index()];
        stats.reward.fetch_add(result as u64, Ordering::Relaxed);
        stats.visits.fetch_add(1, Ordering::Relaxed);
        self.visits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn rollout(&self, you: &SnakeId, stats: &mut SearchDepthStats) -> u32 {
        rollout_from(
            self.board,
            you,
            stats,
            &mut rand::rng(),
            self.board.get_length(you),
            self.rollout_options(),
        )
    }

    fn rollout_with_rng(
        &self,
        you: &SnakeId,
        stats: &mut SearchDepthStats,
        rng: &mut impl Rng,
        root_length: u16,
    ) -> u32 {
        rollout_from(
            self.board,
            you,
            stats,
            rng,
            root_length,
            self.rollout_options(),
        )
    }
}

fn rollout_from(
    mut board: CellBoard4Snakes11x11,
    you: &SnakeId,
    stats: &mut SearchDepthStats,
    rng: &mut impl Rng,
    root_length: u16,
    options: RolloutOptions,
) -> u32 {
    #[cfg(feature = "tracy")]
    let _tracy_span = tracy_client::span!("rollout_from");
    let mut depth = 0;
    let mut food = if options.food_guidance {
        board.get_all_food_as_positions()
    } else {
        ArrayVec::new()
    };

    while !board.is_over() && board.get_health(you) > 0 && depth < options.depth {
        let moves = sample_rollout_moves(&board, &food, rng, options.food_guidance);
        board = {
            #[cfg(feature = "tracy")]
            let _tracy_span = tracy_client::span!("rollout simulation");
            board.simulate_single_action(&moves).1
        };
        // Rollout simulation removes eaten food and does not spawn replacement food.
        // Updating the small food list avoids scanning every board cell each step.
        if options.food_guidance {
            food.retain(|pos| board.is_food(&board.native_from_position(*pos)));
        }
        depth += 1;
    }

    stats.max_rollout_depth = stats.max_rollout_depth.max(depth);
    if depth == options.depth && !board.is_over() && board.get_health(you) > 0 {
        stats.rollout_depth_limit_hits += 1;
    }

    if board.get_health(you) == 0 {
        0
    } else if board.is_over() && board.get_winner().is_some_and(|winner| winner == *you) {
        WIN_REWARD
    } else {
        let gained = u32::from(board.get_length(you).saturating_sub(root_length));
        // The rollout already tracks exactly which cells still hold food, so the evaluator can
        // use that list instead of rescanning the board. When food guidance is off the list is
        // never populated, so fall back to the board's own list.
        let score = if options.food_guidance {
            evaluate_board_with_food(&board, you, &food)
        } else {
            evaluate_board(&board, you)
        };
        (leaf_reward(score) + gained * options.food_gain_reward).min(WIN_REWARD - 1)
    }
}

/// Own reservations for the entire selected path, including panic cleanup.
struct SearchPath(ArrayVec<(Arc<Node>, Move), TREE_PATH_CAPACITY>);

impl Drop for SearchPath {
    fn drop(&mut self) {
        for (node, mv) in &self.0 {
            node.own_moves[mv.as_index()]
                .in_flight
                .fetch_sub(1, Ordering::Relaxed);
        }
    }
}

fn search_iteration(
    root: &Arc<Node>,
    you: &SnakeId,
    rng: &mut impl Rng,
    stats: &mut SearchDepthStats,
) {
    #[cfg(feature = "tracy")]
    let _tracy_span = tracy_client::span!("search_iteration");
    const EXPLORATION: f64 = 1.0;
    stats.iterations += 1;
    let mut path = SearchPath(ArrayVec::new());
    let mut node = Arc::clone(root);
    let root_length = root.board.get_length(you);
    let result = loop {
        if node.board.is_over() || node.board.get_health(you) == 0 {
            break node.rollout_with_rng(you, stats, rng, root_length);
        }
        if path.0.len() == root.tree_depth {
            stats.tree_depth_limit_hits += 1;
            break node.rollout_with_rng(you, stats, rng, root_length);
        }
        let Some(own_move) = node.select_own_move(*you, EXPLORATION) else {
            break node.rollout_with_rng(you, stats, rng, root_length);
        };
        // Reserve before expansion or simulation; the guard also releases on unwind.
        node.own_moves[own_move.as_index()]
            .in_flight
            .fetch_add(1, Ordering::Relaxed);
        path.0.push((node, own_move));
        let visited = &path.0.last().unwrap().0;
        let action = visited.sample_joint_action(*you, own_move, rng);
        let child = visited.child_for_action(&action);

        match child {
            ChildResult::Existing(next) => node = next,
            ChildResult::Expanded(next) => {
                break next.rollout_with_rng(you, stats, rng, root_length);
            }
            ChildResult::Unstored(next_board) => {
                break rollout_from(
                    next_board,
                    you,
                    stats,
                    rng,
                    root_length,
                    root.rollout_options(),
                );
            }
        }
    };

    stats.max_tree_depth = stats.max_tree_depth.max(path.0.len());
    for (visited, own_move) in &path.0 {
        visited.record(*own_move, result);
    }
}

/// Perform one search iteration, mainly useful for profiling the search.
pub fn search_once(root: &Arc<Node>, you: &SnakeId, stats: &mut SearchDepthStats) {
    root.prepare_escape_guard(*you);
    search_iteration(root, you, &mut rand::rng(), stats);
}

/// One iteration with caller-owned RNG, for reproducible tactical checks and comparisons.
pub fn search_once_with_rng(
    root: &Arc<Node>,
    you: &SnakeId,
    stats: &mut SearchDepthStats,
    rng: &mut impl Rng,
) {
    root.prepare_escape_guard(*you);
    search_iteration(root, you, rng, stats);
}

/// Benchmark access to the existing rollout hot paths without changing production behavior.
#[cfg(feature = "bench")]
pub mod bench {
    use super::*;

    pub fn prepare_root(node: &Node, you: SnakeId) {
        node.prepare_escape_guard(you);
    }

    pub fn search_with_rng(
        root: &Arc<Node>,
        you: &SnakeId,
        stats: &mut SearchDepthStats,
        rng: &mut impl Rng,
        iterations: u64,
    ) {
        root.prepare_escape_guard(*you);
        for _ in 0..iterations {
            search_iteration(root, you, rng, stats);
        }
    }

    pub fn rollout_with_rng(
        node: &Node,
        you: &SnakeId,
        stats: &mut SearchDepthStats,
        rng: &mut impl Rng,
    ) -> u32 {
        node.rollout_with_rng(you, stats, rng, node.board.get_length(you))
    }

    pub fn sample_rollout_moves(
        board: &CellBoard4Snakes11x11,
        food: &[Position],
        rng: &mut impl Rng,
    ) -> ArrayVec<(SnakeId, Move), 4> {
        super::sample_rollout_moves(board, food, rng, true)
    }
}

struct WorkerCompletion {
    remaining: usize,
    stats: SearchDepthStats,
    panic: Option<Box<dyn std::any::Any + Send>>,
}

/// Callers wait outside Rayon: a worker finishing this search must not execute
/// another game's long-lived job while helping a nested Rayon join.
struct SearchWorkers<F> {
    root: Arc<Node>,
    you: SnakeId,
    stopped: F,
    failed: AtomicBool,
    completion: Mutex<WorkerCompletion>,
    finished: Condvar,
}

fn run_workers(
    root: &Arc<Node>,
    you: &SnakeId,
    workers: usize,
    stopped: impl Fn() -> bool + Send + Sync + 'static,
) -> SearchDepthStats {
    assert!((1..=SEARCH_WORKERS).contains(&workers));
    let group = Arc::new(SearchWorkers {
        root: Arc::clone(root),
        you: *you,
        stopped,
        failed: AtomicBool::new(false),
        completion: Mutex::new(WorkerCompletion {
            remaining: workers,
            stats: SearchDepthStats::default(),
            panic: None,
        }),
        finished: Condvar::new(),
    });
    for _ in 0..workers {
        let group = Arc::clone(&group);
        search_pool().spawn(move || {
            let mut stats = SearchDepthStats::default();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut rng = rand::rng();
                while !group.failed.load(Ordering::Relaxed) && !(group.stopped)() {
                    search_iteration(&group.root, &group.you, &mut rng, &mut stats);
                }
            }));
            if result.is_err() {
                group.failed.store(true, Ordering::Relaxed);
            }
            // Merge once per worker, never in the iteration hot path.
            let mut completion = group.completion.lock().unwrap();
            completion.stats.iterations += stats.iterations;
            completion.stats.max_tree_depth =
                completion.stats.max_tree_depth.max(stats.max_tree_depth);
            completion.stats.tree_depth_limit_hits += stats.tree_depth_limit_hits;
            completion.stats.max_rollout_depth = completion
                .stats
                .max_rollout_depth
                .max(stats.max_rollout_depth);
            completion.stats.rollout_depth_limit_hits += stats.rollout_depth_limit_hits;
            if let Err(panic) = result {
                completion.panic.get_or_insert(panic);
            }
            completion.remaining -= 1;
            // No tree mutations occur after reporting completion.
            if completion.remaining == 0 {
                group.finished.notify_one();
            }
        });
    }
    let mut completion = group.completion.lock().unwrap();
    while completion.remaining != 0 {
        completion = group.finished.wait(completion).unwrap();
    }
    if let Some(panic) = completion.panic.take() {
        // Release the completion lock before propagating the failure to the caller.
        drop(completion);
        std::panic::resume_unwind(panic);
    }
    std::mem::take(&mut completion.stats)
}

/// Run a bounded search on the persistent pool; workers join before returning.
/// The deadline includes pool scheduling and root preparation, but not tree destruction.
pub fn search_for(
    root: &Arc<Node>,
    you: &SnakeId,
    duration: Duration,
    workers: usize,
) -> SearchDepthStats {
    let deadline = Instant::now() + duration;
    root.prepare_escape_guard(*you);
    run_workers(root, you, workers, move || Instant::now() >= deadline)
}

pub fn mcts_search(root: Arc<Node>, you: &SnakeId, stop: Arc<AtomicBool>) {
    mcts_search_with_publish(root, you, stop, || {});
}

/// Publish only after the search loop stops successfully, before logging or tree cleanup.
pub fn mcts_search_with_publish(
    root: Arc<Node>,
    you: &SnakeId,
    stop: Arc<AtomicBool>,
    publish: impl FnOnce(),
) {
    root.prepare_escape_guard(*you);
    let stats = run_workers(&root, you, SEARCH_WORKERS, move || {
        stop.load(Ordering::Relaxed)
    });
    // All workers finish tree mutations before publishing or retaining nodes.
    publish();
    info!(
        workers = SEARCH_WORKERS,
        iterations = stats.iterations,
        max_tree_depth = root.tree_depth,
        observed_max_tree_depth = stats.max_tree_depth,
        max_tree_depth_reached = stats.max_tree_depth == root.tree_depth,
        tree_depth_cap_truncated_search = stats.tree_depth_limit_hits > 0,
        tree_depth_limit_hits = stats.tree_depth_limit_hits,
        max_rollout_depth = root.rollout_depth,
        observed_max_rollout_depth = stats.max_rollout_depth,
        max_rollout_depth_reached = stats.max_rollout_depth == root.rollout_depth,
        rollout_depth_cap_truncated_search = stats.rollout_depth_limit_hits > 0,
        rollout_depth_limit_hits = stats.rollout_depth_limit_hits,
        "MCTS search depth telemetry"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use battlesnake_game_types::wire_representation::Position;
    use battlesnake_game_types::{types::build_snake_id_map, wire_representation::Game};
    use rand::SeedableRng;
    use std::{thread, time::Duration};

    static SEARCH_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn turn33() -> (CellBoard4Snakes11x11, SnakeId, SnakeId) {
        let game: Game = serde_json::from_str(include_str!("../fixtures/turn33-food.json"))
            .expect("valid turn-33 fixture");
        let ids = build_snake_id_map(&game);
        let board = game.as_cell_board(&ids).expect("valid board");
        (board, ids[&game.you.id], ids[&game.board.snakes[0].id])
    }

    #[test]
    fn child_results_preserve_expansion_rollouts_and_cached_identity() {
        use battlesnake_game_types::types::SimulableGame;
        let (board, you, _) = turn33();
        let legal = board.reasonable_moves_for_each_snake();
        let actions: Vec<_> = board.simulate_with_moves(&legal).take(4).collect();
        assert_eq!(actions.len(), 4);
        for guidance in [false, true] {
            let root = Node::new_root_with_options(board, guidance, 17, 7);
            for (index, (action, expected)) in actions.iter().enumerate() {
                let moves: ArrayVec<_, 4> = action
                    .into_inner()
                    .iter()
                    .enumerate()
                    .filter_map(|(id, mv)| mv.map(|mv| (SnakeId(id as u8), mv)))
                    .collect();
                match root.child_for_action(&moves) {
                    ChildResult::Expanded(child) => {
                        assert!(index < 3);
                        assert_eq!(child.board, *expected);
                        let mut actual_rng = rand::rngs::SmallRng::seed_from_u64(181);
                        let mut reference_rng = actual_rng.clone();
                        let mut actual_stats = SearchDepthStats::default();
                        let mut reference_stats = SearchDepthStats::default();
                        let root_length = board.get_length(&you);
                        let actual = child.rollout_with_rng(
                            &you,
                            &mut actual_stats,
                            &mut actual_rng,
                            root_length,
                        );
                        let reference = rollout_from(
                            *expected,
                            &you,
                            &mut reference_stats,
                            &mut reference_rng,
                            root_length,
                            root.rollout_options(),
                        );
                        assert_eq!(actual, reference);
                        assert_eq!(
                            actual_stats.max_rollout_depth,
                            reference_stats.max_rollout_depth
                        );
                        assert_eq!(
                            actual_stats.rollout_depth_limit_hits,
                            reference_stats.rollout_depth_limit_hits
                        );
                        assert_eq!(actual_rng.random::<u64>(), reference_rng.random::<u64>());
                        let ChildResult::Existing(cached) = root.child_for_action(&moves) else {
                            panic!("cached action must be reused");
                        };
                        assert!(Arc::ptr_eq(&child, &cached));
                    }
                    ChildResult::Unstored(next) => {
                        assert_eq!(index, 3);
                        assert_eq!(next, *expected);
                    }
                    ChildResult::Existing(_) => panic!("new action should not be cached"),
                }
            }
        }
    }

    #[test]
    fn tree_cache_reuses_only_an_exact_same_length_child() {
        let (board, you, _) = turn33();
        let root = Arc::new(Node::new_root(board));
        let mut rng = rand::rngs::SmallRng::seed_from_u64(19);
        let mut stats = SearchDepthStats::default();
        for _ in 0..500 {
            search_iteration(&root, &you, &mut rng, &mut stats);
        }
        let (action, child) = root
            .children
            .read()
            .unwrap()
            .iter()
            .find(|(_, child)| child.visits() > 0)
            .map(|(action, child)| (*action, Arc::clone(child)))
            .expect("search should revisit an expanded child");
        let chosen = action.into_inner()[you.as_usize()].unwrap();
        let cache = SearchTreeCache::after_move(&root, you, chosen);
        assert!(cache.candidate_count() > 0);
        assert!(cache.match_observed(&board, you).is_none());

        let cache = SearchTreeCache::after_move(&root, you, chosen);
        assert!(Arc::ptr_eq(
            &cache.match_observed(&child.board, you).unwrap(),
            &child
        ));

        let mut cache = SearchTreeCache::after_move(&root, you, chosen);
        cache.root_length = cache.root_length.saturating_add(1);
        assert!(cache.match_observed(&child.board, you).is_none());
    }

    fn policy_fixture(
        food: Vec<Position>,
        own_health: u8,
        contested_head: bool,
    ) -> (CellBoard4Snakes11x11, SnakeId, SnakeId) {
        let mut game: Game = serde_json::from_str(include_str!("../fixtures/turn33-food.json"))
            .expect("valid fixture");
        let own_wire_id = game.you.id.clone();
        let own_head = Position { x: 5, y: 5 };
        let own_neck = Position { x: 5, y: 4 };
        let other_head = if contested_head {
            Position { x: 5, y: 7 }
        } else {
            Position { x: 2, y: 9 }
        };
        let other_neck = Position {
            x: other_head.x,
            y: other_head.y + 1,
        };
        for snake in &mut game.board.snakes {
            if snake.id == own_wire_id {
                snake.health = i32::from(own_health);
                snake.head = own_head;
                snake.body = vec![own_head, own_neck].into();
            } else {
                snake.head = other_head;
                snake.body = vec![other_head, other_neck].into();
            }
        }
        game.board.food = food;
        game.you.health = i32::from(own_health);
        game.you.head = own_head;
        game.you.body = vec![own_head, own_neck].into();
        let ids = build_snake_id_map(&game);
        let board = game.as_cell_board(&ids).expect("valid policy board");
        (
            board,
            ids[&own_wire_id],
            ids.values()
                .copied()
                .find(|id| *id != ids[&own_wire_id])
                .unwrap(),
        )
    }

    /// Build an 11x11 board from explicit, already-adjacent body chains. The first
    /// position is the head. `opponents` is a slice of (body, health) specs.
    fn board_from_specs(
        own_body: &[Position],
        own_health: i32,
        opponents: &[(&[Position], i32)],
        food: Vec<Position>,
    ) -> (CellBoard4Snakes11x11, SnakeId, Vec<SnakeId>) {
        use battlesnake_game_types::wire_representation::BattleSnake;

        let mut game: Game = serde_json::from_str(include_str!("../fixtures/turn33-food.json"))
            .expect("valid fixture");
        let own_wire_id = game.you.id.clone();
        let template = game
            .board
            .snakes
            .iter()
            .find(|snake| snake.id != own_wire_id)
            .expect("fixture has an opponent")
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
        let own = ids[&own_wire_id];
        let opponent_ids = (0..opponents.len())
            .map(|index| ids[&format!("gs_opponent_{index}")])
            .collect();
        (board, own, opponent_ids)
    }

    #[test]
    fn equal_length_shared_head_destination_is_pruned() {
        // Own head (5, 5), opponent head (5, 7): both can enter (5, 6). Equal lengths
        // mean the simulator kills both of us, so Up must leave the tree candidate set.
        let (board, you, _) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(&[Position::new(5, 7), Position::new(5, 8)], 100)],
            Vec::new(),
        );
        let node = Node::new_root(board);
        let legal = node.legal_own_moves(you).unwrap();
        assert!(legal.contains(&Move::Up));
        let tree = node.tree_own_moves(you).unwrap();
        assert!(!tree.contains(&Move::Up));
        assert!(tree.contains(&Move::Left));
        assert!(tree.contains(&Move::Right));
        assert!(!tree.as_slice().is_empty());
    }

    #[test]
    fn longer_opponent_shared_head_destination_is_pruned() {
        let (board, you, _) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(
                &[
                    Position::new(5, 7),
                    Position::new(5, 8),
                    Position::new(5, 9),
                ],
                100,
            )],
            Vec::new(),
        );
        let node = Node::new_root(board);
        let tree = node.tree_own_moves(you).unwrap();
        assert!(!tree.contains(&Move::Up));
        assert!(tree.contains(&Move::Left));
    }

    #[test]
    fn shorter_opponent_shared_head_destination_is_retained() {
        // We are the longer snake, so we win the shared square and keep the move.
        let (board, you, _) = board_from_specs(
            &[
                Position::new(5, 5),
                Position::new(5, 4),
                Position::new(5, 3),
            ],
            100,
            &[(&[Position::new(5, 7), Position::new(5, 8)], 100)],
            Vec::new(),
        );
        let node = Node::new_root(board);
        assert!(node.tree_own_moves(you).unwrap().contains(&Move::Up));
    }

    #[test]
    fn uncontested_destinations_keep_the_full_reasonable_set() {
        let (board, you, _) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(&[Position::new(2, 9), Position::new(2, 10)], 100)],
            Vec::new(),
        );
        let node = Node::new_root(board);
        let legal = node.legal_own_moves(you).unwrap();
        let tree = node.tree_own_moves(you).unwrap();
        assert_eq!(tree.as_slice(), legal.as_slice());
        assert!(tree.contains(&Move::Up));
    }

    #[test]
    fn food_growth_keeps_a_starving_challenger_in_the_head_to_head() {
        // Equal lengths, opponent at 1 HP, and the shared square is food. Eating keeps
        // the opponent alive and grows both snakes, so the tie still kills us. Without
        // the food the opponent starves and never contests the square.
        let own = [Position::new(5, 5), Position::new(5, 4)];
        let opponent = [Position::new(5, 7), Position::new(5, 8)];
        let shared = Position::new(5, 6);

        let (fed_board, fed_you, fed_opponents) =
            board_from_specs(&own, 100, &[(&opponent, 1)], vec![shared]);
        let fed_node = Node::new_root(fed_board);
        let fed_opponent = fed_opponents[0];
        assert!(
            fed_node
                .tree_own_moves(fed_you)
                .unwrap()
                .contains(&Move::Left)
        );
        assert!(
            !fed_node
                .tree_own_moves(fed_you)
                .unwrap()
                .contains(&Move::Up)
        );

        let (_, fed_next) =
            fed_board.simulate_single_action(&[(fed_you, Move::Up), (fed_opponent, Move::Down)]);
        assert_eq!(
            fed_next.get_health_i64(&fed_you),
            0,
            "both grew to equal length, so both die"
        );
        assert_eq!(fed_next.get_health_i64(&fed_opponent), 0);

        let (hungry_board, hungry_you, hungry_opponents) =
            board_from_specs(&own, 100, &[(&opponent, 1)], Vec::new());
        let hungry_node = Node::new_root(hungry_board);
        assert!(
            hungry_node
                .tree_own_moves(hungry_you)
                .unwrap()
                .contains(&Move::Up)
        );

        let (_, hungry_next) = hungry_board
            .simulate_single_action(&[(hungry_you, Move::Up), (hungry_opponents[0], Move::Down)]);
        assert_eq!(
            hungry_next.get_health_i64(&hungry_opponents[0]),
            0,
            "without food the 1 HP opponent starves"
        );
        assert!(hungry_next.get_health_i64(&hungry_you) > 0);
    }

    #[test]
    fn opponent_that_dies_in_hazard_does_not_prune_shared_destination() {
        use battlesnake_game_types::types::HazardSettableGame;

        let shared = Position::new(5, 6);
        let (template, _, _) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(&[Position::new(5, 7), Position::new(5, 8)], 100)],
            Vec::new(),
        );
        let opponent_health = i32::from(template.get_hazard_damage()) + 1;

        // Reset to exactly one move's worth of health above lethal hazard damage.
        // The opponent's move to the shared square is geometrically reasonable but dies
        // during per-snake state generation, before head-to-head resolution.
        let (mut board, you, opponents) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(&[Position::new(5, 7), Position::new(5, 8)], opponent_health)],
            Vec::new(),
        );
        board.set_hazard(board.native_from_position(shared));
        let node = Node::new_root(board);
        assert!(node.tree_own_moves(you).unwrap().contains(&Move::Up));

        let (_, next) =
            board.simulate_single_action(&[(you, Move::Up), (opponents[0], Move::Down)]);
        assert!(next.get_health_i64(&you) > 0);
        assert_eq!(next.get_health_i64(&opponents[0]), 0);
    }

    #[test]
    fn all_contested_moves_fall_back_to_the_full_reasonable_set() {
        // Own head (0, 5) has a spine going down, leaving only Up (0, 6) and Right
        // (1, 5) as reasonable moves (Down is the neck, Left is off-board). Two
        // equal-length opponents can each enter one of them, so naive filtering would
        // leave nothing. The fallback must keep both moves deterministically.
        let (board, you, _) = board_from_specs(
            &[
                Position::new(0, 5),
                Position::new(0, 4),
                Position::new(0, 3),
            ],
            100,
            &[
                (
                    &[
                        Position::new(1, 6),
                        Position::new(2, 6),
                        Position::new(3, 6),
                    ],
                    100,
                ),
                (
                    &[
                        Position::new(1, 4),
                        Position::new(2, 4),
                        Position::new(3, 4),
                    ],
                    100,
                ),
            ],
            Vec::new(),
        );
        let node = Node::new_root(board);
        let legal = node.legal_own_moves(you).unwrap();
        assert_eq!(legal.as_slice(), &[Move::Up, Move::Right]);
        let tree = node.tree_own_moves(you).unwrap();
        assert_eq!(tree.as_slice(), legal.as_slice());
        assert_eq!(
            node.tree_own_moves(you).unwrap().as_slice(),
            tree.as_slice()
        );
    }

    #[test]
    fn tree_opponents_avoid_losing_contests_but_keep_every_safe_reply() {
        let (board, you, opponents) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(&[Position::new(5, 7), Position::new(5, 8)], 100)],
            Vec::new(),
        );
        let opponent = opponents[0];
        let full = board.reasonable_moves_for_each_snake();
        let node = Node::new_root(board);
        let cache = node.move_cache();

        // The sampling masks used by opponents and rollouts are untouched by pruning.
        for (id, legal) in &full {
            let cached = cache
                .masks
                .iter()
                .find(|(cached_id, _)| cached_id == id)
                .map(|(_, mask)| *mask)
                .unwrap();
            let expected = legal.iter().fold(0u8, |mask, mv| mask | 1 << mv.as_index());
            assert_eq!(cached, expected);
        }
        assert!(
            cache.tree_masks[you.as_usize()] & (1 << Move::Up.as_index()) == 0,
            "our tree mask must be pruned"
        );
        let cached_opponent = cache
            .masks
            .iter()
            .find(|(id, _)| *id == opponent)
            .map(|(_, mask)| *mask)
            .unwrap();
        assert!(
            cached_opponent & (1 << Move::Down.as_index()) != 0,
            "the opponent's full move mask still contains the contested move"
        );

        let mask = cache.tree_masks[opponent.as_usize()];
        assert_eq!(mask & (1 << Move::Down.as_index()), 0);
        let expected_total: u32 = mask_bits(mask)
            .map(|m| u32::from(cache.policies[opponent.as_usize()][m.as_index()]))
            .sum();
        assert_eq!(cache.policy_totals[opponent.as_usize()], expected_total);
        let mut rng = rand::rngs::SmallRng::seed_from_u64(7);
        let mut observed = 0;
        for _ in 0..4096 {
            let joint = node.sample_joint_action(you, Move::Left, &mut rng);
            let mv = joint.iter().find(|(id, _)| *id == opponent).unwrap().1;
            assert_ne!(mv, Move::Down);
            observed |= 1 << mv.as_index();
        }
        assert_eq!(observed, mask);
    }

    #[test]
    fn best_and_selection_ignore_contested_losing_moves() {
        let (board, you, _) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(&[Position::new(5, 7), Position::new(5, 8)], 100)],
            Vec::new(),
        );
        let node = Node::new_root(board);
        // Even with a decisive reward history, a pruned move is never selected.
        node.record(Move::Up, WIN_REWARD * 10);
        node.record(Move::Left, 1);
        assert_ne!(node.best_move(you), Some(Move::Up));
        assert_ne!(node.select_own_move(you, 1.0), Some(Move::Up));
        assert_eq!(node.best_move(you), Some(Move::Left));
    }

    /// The offset table replaces a per-candidate match, so pin it against the direct geometry:
    /// for every reachable opponent offset, which of our destinations it contests and from which
    /// side it would approach.
    #[test]
    fn move_offset_tables_match_the_direct_head_to_head_geometry() {
        for ox in -3..=3 {
            for oy in -3..=3 {
                for (index, (mx, my)) in MOVE_OFFSETS.into_iter().enumerate() {
                    let (dx, dy) = (ox - mx, oy - my);
                    if dx.abs() + dy.abs() != 1 {
                        continue;
                    }
                    let approach = if dx == 1 {
                        Move::Left
                    } else if dx == -1 {
                        Move::Right
                    } else if dy == 1 {
                        Move::Down
                    } else {
                        Move::Up
                    };
                    assert_eq!(
                        MOVE_FROM_OFFSET[(dx + 1) as usize * 3 + (dy + 1) as usize],
                        approach.as_index(),
                        "opponent at ({ox}, {oy}), our move index {index}"
                    );
                }
                for (index, mv) in Move::all().into_iter().enumerate() {
                    assert_eq!(MOVE_OFFSETS[index], (mv.dx(), mv.dy()), "{mv:?}");
                }
            }
        }
    }

    #[test]
    fn leaf_rewards_stay_between_terminal_outcomes_and_preserve_order() {
        let mut previous = 0;
        for score in 0..=u16::MAX {
            let reward = leaf_reward(score);
            assert!(reward > 0 && reward < WIN_REWARD);
            assert!(reward >= previous);
            previous = reward;
        }
        assert!(leaf_reward(1600) > leaf_reward(800));
    }

    #[test]
    fn one_lucky_win_does_not_dominate_consistently_good_leaves() {
        let (board, you, _) = turn33();
        let node = Node::new_root(board);
        let moves: Vec<_> = node.legal_own_moves(you).unwrap().into_iter().collect();
        assert!(moves.len() >= 2);
        for &mv in &moves {
            for i in 0..1000 {
                let reward = if mv == moves[1] {
                    leaf_reward(800)
                } else if mv == moves[0] && i == 0 {
                    WIN_REWARD
                } else {
                    0
                };
                node.record(mv, reward);
            }
        }
        assert_eq!(node.select_own_move(you, 1.0), Some(moves[1]));
    }

    #[test]
    fn rollout_uses_terminal_rewards_for_winner_and_dead_snake() {
        let mut game: Game =
            serde_json::from_str(include_str!("../fixtures/turn33-food.json")).unwrap();
        game.board.snakes.retain(|snake| snake.id == game.you.id);
        let ids = build_snake_id_map(&game);
        let board = game.as_cell_board(&ids).unwrap();
        let you = ids[&game.you.id];
        let node = Node::new_root(board);
        assert_eq!(
            node.rollout(&you, &mut SearchDepthStats::default()),
            WIN_REWARD
        );
        assert_eq!(
            node.rollout(&SnakeId(3), &mut SearchDepthStats::default()),
            0
        );
    }

    #[test]
    fn opponent_moves_are_sampled_independently_of_our_move() {
        let (board, you, opponent) = turn33();
        let node = Node::new_root(board);
        assert_ne!(opponent, SnakeId(0));
        let mut up_rng = rand::rngs::SmallRng::seed_from_u64(12);
        let mut down_rng = rand::rngs::SmallRng::seed_from_u64(12);
        for _ in 0..100 {
            let up = node.sample_joint_action(you, Move::Up, &mut up_rng);
            let down = node.sample_joint_action(you, Move::Down, &mut down_rng);
            assert_eq!(
                up.iter().find(|(id, _)| *id == opponent),
                down.iter().find(|(id, _)| *id == opponent)
            );
        }
    }

    fn search_with_seed(
        board: CellBoard4Snakes11x11,
        you: SnakeId,
        seed: u64,
        iterations: usize,
    ) -> Move {
        let root = Arc::new(Node::new_root(board));
        let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
        let mut stats = SearchDepthStats::default();
        for _ in 0..iterations {
            search_iteration(&root, &you, &mut rng, &mut stats);
        }
        root.best_move(you)
            .expect("search should choose a legal move")
    }

    #[test]
    fn search_takes_or_approaches_uncontested_food_before_starving() {
        // These routes are the only ways to reach food before health reaches zero.
        // Run several fixed RNG streams so a single lucky rollout cannot satisfy the test.
        let cases = [
            (
                [Position::new(5, 5), Position::new(5, 4)],
                2,
                Position::new(5, 6),
                Move::Up,
            ),
            (
                [Position::new(5, 5), Position::new(5, 4)],
                3,
                Position::new(5, 7),
                Move::Up,
            ),
            (
                [Position::new(5, 5), Position::new(4, 5)],
                4,
                Position::new(8, 5),
                Move::Right,
            ),
        ];

        for (own_body, health, food, expected) in cases {
            let (board, you, _) = board_from_specs(
                &own_body,
                health,
                &[(&[Position::new(1, 9), Position::new(1, 10)], 100)],
                vec![food],
            );
            let correct_seeds = (0..4)
                .filter(|seed| search_with_seed(board, you, *seed, 2_000) == expected)
                .count();
            assert!(
                correct_seeds >= 3,
                "expected {expected:?} in at least 3 of 4 seeded searches, got {correct_seeds}"
            );
        }

        // Regression state from game aec87093, turn 83: Up reaches the only nearby
        // food in two moves, while Left sends the snake around the edge at 36 health.
        let own_body = [
            Position::new(10, 1),
            Position::new(10, 0),
            Position::new(9, 0),
            Position::new(8, 0),
            Position::new(7, 0),
            Position::new(6, 0),
        ];
        let long_opponent = [
            Position::new(5, 2),
            Position::new(4, 2),
            Position::new(3, 2),
            Position::new(2, 2),
            Position::new(1, 2),
            Position::new(1, 3),
            Position::new(1, 4),
            Position::new(1, 5),
            Position::new(1, 6),
            Position::new(1, 7),
            Position::new(2, 7),
            Position::new(3, 7),
            Position::new(4, 7),
            Position::new(4, 6),
            Position::new(5, 6),
            Position::new(5, 7),
        ];
        let (board, you, _) = board_from_specs(
            &own_body,
            36,
            &[(&long_opponent, 93)],
            vec![
                Position::new(3, 10),
                Position::new(5, 1),
                Position::new(10, 3),
            ],
        );
        let correct_seeds = (0..4)
            .filter(|seed| search_with_seed(board, you, *seed, 2_000) == Move::Up)
            .count();
        assert!(
            correct_seeds >= 3,
            "turn-83 regression: expected Up in at least 3 of 4 seeded searches, got {correct_seeds}"
        );
    }

    #[test]
    fn hungry_opponent_policy_favors_food_and_keeps_all_moves_supported() {
        let food_pos = Position { x: 4, y: 5 };
        let (board, snake, _) = policy_fixture(vec![food_pos], 1, false);
        let moves = board.reasonable_moves_for_each_snake();
        let legal = moves.iter().find(|(id, _)| *id == snake).unwrap().1;
        let weights = move_weights(&board, snake, legal);
        assert!(weights[Move::Left.as_index()] > weights[Move::Right.as_index()]);
        assert!(legal.iter().all(|mv| weights[mv.as_index()] > 0));
    }

    #[test]
    fn healthy_tree_selection_prior_favors_a_safe_food_move() {
        let (board, snake, _) = policy_fixture(vec![Position::new(4, 5)], 100, false);
        let node = Node::new_root(board);
        let candidates = node.tree_own_moves(snake).unwrap();
        assert!(candidates.contains(&Move::Left));

        // Give all actions identical evidence; the policy prior should break the tie
        // toward food while preserving the ordinary UCB value for every candidate.
        for mv in candidates {
            node.record(mv, 500);
        }
        assert_eq!(node.select_own_move(snake, 1.0), Some(Move::Left));
        assert_eq!(node.best_move(snake), Some(Move::Left));
    }

    #[test]
    fn rollout_policy_samples_safe_food_moves_more_often() {
        let (with_food, snake, _) = policy_fixture(vec![Position::new(4, 5)], 100, false);
        let (without_food, _, _) = policy_fixture(Vec::new(), 100, false);
        let mut food_rng = rand::rngs::SmallRng::seed_from_u64(7);
        let mut baseline_rng = rand::rngs::SmallRng::seed_from_u64(7);
        let mut foodward_with_food = 0;
        let mut foodward_without_food = 0;
        let with_food_positions = with_food.get_all_food_as_positions();
        let without_food_positions = without_food.get_all_food_as_positions();

        for _ in 0..10_000 {
            foodward_with_food += u32::from(
                sample_rollout_moves(&with_food, &with_food_positions, &mut food_rng, true)
                    .iter()
                    .find(|(id, _)| *id == snake)
                    .is_some_and(|(_, mv)| *mv == Move::Left),
            );
            foodward_without_food += u32::from(
                sample_rollout_moves(
                    &without_food,
                    &without_food_positions,
                    &mut baseline_rng,
                    true,
                )
                .iter()
                .find(|(id, _)| *id == snake)
                .is_some_and(|(_, mv)| *mv == Move::Left),
            );
        }

        assert!(
            foodward_with_food > foodward_without_food + 250,
            "expected rollout to prefer the safe food move: with={foodward_with_food}, without={foodward_without_food}"
        );
    }

    #[test]
    fn rollout_food_list_matches_simulated_board_after_eating() {
        let (mut board, you, _) = board_from_specs(
            &[Position::new(5, 5), Position::new(5, 4)],
            100,
            &[(&[Position::new(2, 9), Position::new(2, 10)], 100)],
            vec![Position::new(5, 6), Position::new(0, 0)],
        );
        let mut food = board.get_all_food_as_positions();
        let eat: ArrayVec<_, 4> = board
            .reasonable_moves_for_each_snake()
            .iter()
            .map(|(snake, moves)| (*snake, if *snake == you { Move::Up } else { moves[0] }))
            .collect();
        board = board.simulate_single_action(&eat).1;
        food.retain(|pos| board.is_food(&board.native_from_position(*pos)));
        assert_eq!(food.len(), 1, "the adjacent food should have been eaten");
        assert_eq!(
            food.as_slice(),
            board.get_all_food_as_positions().as_slice()
        );

        let mut rng = rand::rngs::SmallRng::seed_from_u64(29);
        for _ in 0..64 {
            if board.is_over() {
                break;
            }
            let moves = sample_rollout_moves(&board, &food, &mut rng, true);
            board = board.simulate_single_action(&moves).1;
            food.retain(|pos| board.is_food(&board.native_from_position(*pos)));
            assert_eq!(
                food.as_slice(),
                board.get_all_food_as_positions().as_slice()
            );
        }
    }

    #[test]
    fn equal_or_larger_shared_head_square_is_downweighted() {
        let (board, snake, _) = policy_fixture(Vec::new(), 100, true);
        let moves = board.reasonable_moves_for_each_snake();
        let legal = moves.iter().find(|(id, _)| *id == snake).unwrap().1;
        let weights = move_weights(&board, snake, legal);
        assert!(legal.contains(&Move::Up));
        assert!(weights[Move::Up.as_index()] < weights[Move::Left.as_index()]);
        assert!(legal.iter().all(|mv| weights[mv.as_index()] > 0));
    }

    #[test]
    fn no_food_and_empty_move_lists_have_safe_fallbacks() {
        let (board, snake, _) = policy_fixture(Vec::new(), 100, false);
        let moves = board.reasonable_moves_for_each_snake();
        let legal = moves.iter().find(|(id, _)| *id == snake).unwrap().1;
        let weights = move_weights(&board, snake, legal);
        assert!(legal.iter().all(|mv| weights[mv.as_index()] > 0));

        let mut rng = rand::rngs::SmallRng::seed_from_u64(31);
        assert_eq!(sample_policy_move(0, [0; 4], 0, &mut rng), Move::Up);
    }

    #[test]
    fn uniform_component_keeps_zero_weight_moves_sampleable() {
        let mask: u8 = [Move::Up, Move::Left, Move::Right]
            .iter()
            .fold(0, |mask, mv| mask | 1 << mv.as_index());
        let mut weights = [0; 4];
        weights[Move::Up.as_index()] = 20;
        let mut rng = rand::rngs::SmallRng::seed_from_u64(44);
        let mut counts = [0; 4];
        for _ in 0..6000 {
            let mv = sample_policy_move(mask, weights, 20, &mut rng);
            assert!(mask & (1 << mv.as_index()) != 0);
            counts[mv.as_index()] += 1;
        }
        assert!(counts[Move::Up.as_index()] > 4500);
        assert!((200..650).contains(&counts[Move::Left.as_index()]));
        assert!((200..650).contains(&counts[Move::Right.as_index()]));
    }

    /// Build the target for an arbitrary cell, so the feasibility check can be exercised on
    /// off-board and non-adjacent squares that `describe_move` never produces.
    fn target_at(board: &CellBoard4Snakes11x11, position: Position) -> MoveTarget<u8> {
        if board.off_board(position) {
            return MoveTarget {
                destination: CellIndex::from_usize(0),
                on_board: false,
                is_food: false,
                is_hazard: false,
                is_own_neck: false,
            };
        }
        let native = board.native_from_position(position);
        MoveTarget {
            destination: native,
            on_board: true,
            is_food: board.is_food(&native),
            is_hazard: board.is_hazard(&native),
            is_own_neck: false,
        }
    }

    #[test]
    fn infeasible_challengers_and_short_snake_reversions_are_not_safe() {
        use battlesnake_game_types::types::HazardSettableGame;
        let (mut board, snake, _) = policy_fixture(Vec::new(), 100, false);
        let head = board.get_head_as_position(&snake);
        let neck = head.add_vec(Move::Down.to_vector());
        assert!(!is_feasible_destination(
            &board,
            snake,
            target_at(&board, neck),
            100
        ));
        assert!(!is_feasible_destination(
            &board,
            snake,
            target_at(&board, Position::new(5, 11)),
            100
        ));
        let destination = head.add_vec(Move::Left.to_vector());
        let native = board.native_from_position(destination);
        board.set_hazard(native);
        let damage = i64::from(board.get_hazard_damage());
        assert!(!is_feasible_destination(
            &board,
            snake,
            target_at(&board, destination),
            damage + 1
        ));
        assert!(is_feasible_destination(
            &board,
            snake,
            target_at(&board, destination),
            damage + 2
        ));
        let node = Node::new_root(board);
        let weights = &node.move_cache().policies[snake.as_usize()];
        assert_eq!(weights[Move::Down.as_index()], 1);
    }

    #[test]
    fn policy_weights_handle_off_board_forced_fallback() {
        // A fallback list can contain an off-board move; use a top-edge snake from a fixture.
        let mut game: Game =
            serde_json::from_str(include_str!("../fixtures/turn33-food.json")).unwrap();
        game.board.snakes.retain(|s| s.id == game.you.id);
        game.you.head = Position::new(5, 10);
        game.you.body = vec![game.you.head, Position::new(5, 9)].into();
        game.board.snakes[0] = game.you.clone();
        let ids = build_snake_id_map(&game);
        let edge_board = game.as_cell_board(&ids).unwrap();
        let snake = ids[&game.you.id];
        let fallback: MoveArray = [Move::Up].into_iter().collect();
        let weights = move_weights(&edge_board, snake, fallback);
        assert_eq!(weights[Move::Up.as_index()], 1);
    }

    #[test]
    fn move_statistics_aggregate_across_opponent_responses() {
        let (board, you, _) = turn33();
        let node = Node::new_root(board);
        node.record(Move::Up, 1000);
        node.record(Move::Up, 0);
        node.record(Move::Right, 1000);
        assert_eq!(
            node.own_moves[Move::Up.as_index()]
                .visits
                .load(Ordering::Relaxed),
            2
        );
        assert_eq!(
            node.own_moves[Move::Up.as_index()]
                .reward
                .load(Ordering::Relaxed),
            1000
        );
        assert_eq!(node.best_move(you), Some(Move::Up));
    }

    #[test]
    fn concurrent_expansion_keeps_one_child_per_action() {
        let (board, you, _) = turn33();
        for _ in 0..32 {
            let root = Arc::new(Node::new_root(board));
            let mut rng = rand::rng();
            let mv = root.select_own_move(you, 1.0).unwrap();
            let action = root.sample_joint_action(you, mv, &mut rng);
            let barrier = std::sync::Barrier::new(SEARCH_WORKERS);
            thread::scope(|scope| {
                let handles: Vec<_> = (0..SEARCH_WORKERS)
                    .map(|_| {
                        scope.spawn(|| {
                            barrier.wait();
                            match root.child_for_action(&action) {
                                ChildResult::Existing(child) | ChildResult::Expanded(child) => {
                                    child
                                }
                                ChildResult::Unstored(_) => {
                                    panic!("one action fits the child limit")
                                }
                            }
                        })
                    })
                    .collect();
                let children: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
                for child in &children {
                    assert!(Arc::ptr_eq(child, &children[0]));
                }
                assert_eq!(root.children.read().unwrap().len(), 1);
            });
        }
    }

    fn assert_no_reservations(node: &Node) {
        for stats in &node.own_moves {
            assert_eq!(stats.in_flight.load(Ordering::Relaxed), 0);
        }
        for child in node.children.read().unwrap().values() {
            assert_no_reservations(child);
        }
    }

    #[test]
    fn reservations_release_on_unwind() {
        let (board, _, _) = turn33();
        let root = Arc::new(Node::new_root(board));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut path = SearchPath(ArrayVec::new());
            root.own_moves[0].in_flight.fetch_add(1, Ordering::Relaxed);
            path.0.push((Arc::clone(&root), Move::from_index(0)));
            panic!("simulated search failure");
        }));
        assert!(result.is_err());
        assert_no_reservations(&root);
    }

    #[test]
    fn parallel_search_accounts_for_every_iteration_and_joins_before_publication() {
        let _pool_test = SEARCH_TEST_LOCK.lock().unwrap();
        let (board, you, _) = turn33();
        let root = Arc::new(Node::new_root_with_tree_depth(board, 1));
        let stats = search_for(&root, &you, Duration::from_millis(50), SEARCH_WORKERS);
        assert!(stats.iterations > 0);
        assert_eq!(u64::from(root.visits()), stats.iterations);
        assert_eq!(stats.max_tree_depth, 1);
        assert!(stats.tree_depth_limit_hits > 0);
        assert_no_reservations(&root);
        let cache = SearchTreeCache::after_move(&root, you, root.best_move(you).unwrap());
        assert!(cache.candidate_count() <= SearchTreeCache::MAX_CANDIDATES);

        let stop = Arc::new(AtomicBool::new(true));
        mcts_search_with_publish(Arc::clone(&root), &you, stop, || {
            assert_no_reservations(&root)
        });
        assert_eq!(u64::from(root.visits()), stats.iterations);
    }

    #[test]
    fn worker_failure_stops_siblings_and_the_pool_remains_usable() {
        let _pool_test = SEARCH_TEST_LOCK.lock().unwrap();
        let (board, you, _) = turn33();
        let root = Arc::new(Node::new_root(board));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_workers(&root, &you, SEARCH_WORKERS, || {
                panic!("simulated worker failure")
            });
        }));
        assert!(result.is_err());
        assert_no_reservations(&root);
        let stats = search_for(&root, &you, Duration::from_millis(30), SEARCH_WORKERS);
        assert!(stats.iterations > 0);
        assert_eq!(u64::from(root.visits()), stats.iterations);
        assert_no_reservations(&root);
    }

    #[test]
    fn stopping_one_search_does_not_wait_for_another_games_deadline() {
        let _pool_test = SEARCH_TEST_LOCK.lock().unwrap();
        let (board, you, _) = turn33();
        // Initialize the pool before measuring cancellation.
        search_for(
            &Arc::new(Node::new_root(board)),
            &you,
            Duration::ZERO,
            SEARCH_WORKERS,
        );
        let first_stop = Arc::new(AtomicBool::new(false));
        let second_stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        thread::scope(|scope| {
            let stop = Arc::clone(&first_stop);
            scope.spawn(move || {
                mcts_search_with_publish(Arc::new(Node::new_root(board)), &you, stop, || {
                    tx.send(()).unwrap();
                });
            });
            thread::sleep(Duration::from_millis(20));
            let stop = Arc::clone(&second_stop);
            scope.spawn(move || mcts_search(Arc::new(Node::new_root(board)), &you, stop));
            thread::sleep(Duration::from_millis(30));
            first_stop.store(true, Ordering::Relaxed);
            let result = rx.recv_timeout(Duration::from_millis(100));
            second_stop.store(true, Ordering::Relaxed);
            assert!(
                result.is_ok(),
                "the stopped search must publish while the other remains active"
            );
        });
    }

    #[test]
    fn search_stops_and_produces_a_legal_move() {
        let _pool_test = SEARCH_TEST_LOCK.lock().unwrap();
        let (board, you, _) = turn33();
        let root = Arc::new(Node::new_root(board));
        let stop = Arc::new(AtomicBool::new(false));
        let search_root = Arc::clone(&root);
        let search_stop = Arc::clone(&stop);
        let search = thread::spawn(move || mcts_search(search_root, &you, search_stop));
        thread::sleep(Duration::from_millis(50));
        stop.store(true, Ordering::Relaxed);
        search.join().unwrap();
        assert!(root.visits.load(Ordering::Relaxed) > 0);
        assert!(
            root.legal_own_moves(you)
                .unwrap()
                .contains(&root.best_move(you).unwrap())
        );
    }

    #[test]
    #[ignore = "manual replay diagnostic"]
    fn diagnose_turn33_food_choice() {
        let _pool_test = SEARCH_TEST_LOCK.lock().unwrap();
        let (board, you, _) = turn33();
        for _ in 0..8 {
            let root = Arc::new(Node::new_root(board));
            let stop = Arc::new(AtomicBool::new(false));
            let search_root = Arc::clone(&root);
            let search_stop = Arc::clone(&stop);
            let search = thread::spawn(move || mcts_search(search_root, &you, search_stop));
            thread::sleep(Duration::from_millis(400));
            stop.store(true, Ordering::Relaxed);
            search.join().unwrap();
            println!("{:?}", root.best_move(you));
        }
    }
}
