//! A compact board representation that is efficient for simulation
use crate::compact_representation::core::CellNum as CN;
use crate::impl_common_board_traits;
use crate::types::*;
/// you almost certainly want to use the `convert_from_game` method to
/// cast from a json represention to a `CellBoard`
use crate::types::{NeighborDeterminableGame, SnakeBodyGettableGame};
use crate::wire_representation::Game;
use arrayvec::ArrayVec;
use rand::Rng;
use rand::RngExt;
use std::borrow::Borrow;
use std::error::Error;
use std::fmt::Display;
use tracing::instrument;

use crate::{
    types::{Move, SimulableGame},
    wire_representation::Position,
};

use super::core::CellBoard as CCB;
use super::core::CellIndex;
use super::core::{EvaluateMode, simulate_single_action, simulate_with_moves};
use super::dimensions::{ArcadeMaze, Custom, Dimensions, Fixed, Square};

/// A compact board representation that is significantly faster for simulation than
/// `battlesnake_game_types::wire_representation::Game`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct CellBoard<T: CN, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize> {
    embedded: CCB<T, D, BOARD_SIZE, MAX_SNAKES>,
}

impl_common_board_traits!(CellBoard);

/// 7x7 board with 4 snakes
pub type CellBoard4Snakes7x7 = CellBoard<u8, Square, { 7 * 7 }, 4>;

/// Used to represent the standard 11x11 game with up to 4 snakes.
pub type CellBoard4Snakes11x11 = CellBoard<u8, Square, { 11 * 11 }, 4>;

/// Used to represent the a 15x15 board with up to 4 snakes. This is the biggest board size that
/// can still use u8s
pub type CellBoard8Snakes15x15 = CellBoard<u8, Square, { 15 * 15 }, 8>;

/// Used to represent the largest UI Selectable board with 8 snakes.
pub type CellBoard8Snakes25x25 = CellBoard<u16, Custom, { 25 * 25 }, 8>;

/// Used to represent an absolutely silly game board
pub type CellBoard16Snakes50x50 = CellBoard<u16, Custom, { 50 * 50 }, 16>;

/// The distance at which two heads contest a cell next turn.
///
/// Both snakes move at once, so a contest exists as soon as the cells they can claim overlap,
/// where a snake can claim its own cell (an opponent moving onto it) plus the four cells one step
/// away. With four-way movement that is exactly a Manhattan distance of 2 or less: 2 apart means
/// they meet in the middle, 1 apart means each can move onto the other's head, and 3 apart leaves
/// the closest cells they could move to still 2 apart, so neither can claim the same cell.
/// `contest_radius_equals_sharing_a_cell_next_turn` pins that against a brute force over positions.
pub const CONTEST_RADIUS: usize = 2;

impl<T: CN, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>
    CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>
{
    /// Builds a cellboard from a given game, will return an error if the game doesn't match
    /// the provided BOARD_SIZE or MAX_SNAKES. You are encouraged to use `CellBoard4Snakes11x11`
    /// for the common game layout
    pub fn convert_from_game(game: Game, snake_ids: &SnakeIDMap) -> Result<Self, Box<dyn Error>> {
        if game.game.ruleset.name == "wrapped" {
            return Err("Wrapped games are not supported".into());
        }

        let embedded = CCB::convert_from_game(game, snake_ids)?;
        Ok(CellBoard { embedded })
    }

    fn off_board(&self, new_head: Position) -> bool {
        new_head.x < 0
            || new_head.x >= self.embedded.get_actual_width() as i32
            || new_head.y < 0
            || new_head.y >= self.embedded.get_actual_height() as i32
    }

    /// Return an iterator over all the empty cells on the board
    pub fn get_all_empty(&self) -> impl Iterator<Item = CellIndex<T>> + '_ {
        self.embedded.get_empty_cells()
    }

    /// Simulate exactly one joint action without constructing an iterator.
    pub fn simulate_single_action(&self, moves: &[(SnakeId, Move)]) -> (Action<MAX_SNAKES>, Self) {
        let (action, embedded) =
            simulate_single_action(&self.embedded, moves, EvaluateMode::Standard);
        (action, Self { embedded })
    }

    /// True when a snake could legally move into this cell on the next turn: the cell is not
    /// another head and is either empty or occupied by a tail that vacates this turn.
    #[inline]
    pub fn cell_is_free(&self, pos: CellIndex<T>) -> bool {
        let cell = self.embedded.get_cell(pos);
        !cell.is_head() && (!cell.is_body() || self.embedded.cell_is_single_tail(pos))
    }

    /// Return the surrounding fields not immediately blocked
    pub fn free_neighbors(&self, pos: CellIndex<T>) -> impl Iterator<Item = CellIndex<T>> + '_ {
        let width = self.embedded.get_actual_width();
        let head_pos = pos.into_position(width);

        Move::all_iter()
            .map(move |mv| {
                let new_head = head_pos.add_vec(mv.to_vector());
                let ci = CellIndex::new(new_head, width);

                (new_head, ci)
            })
            .filter(move |(new_head, ci)| !self.off_board(*new_head) && self.cell_is_free(*ci))
            .map(|(_, ci)| ci)
    }

    /// Count free adjacent cells for a valid board cell without constructing positions.
    #[inline]
    pub fn free_neighbor_count(&self, pos: CellIndex<T>) -> u8 {
        let width = usize::from(self.embedded.get_actual_width());
        let height = usize::from(self.embedded.get_actual_height());
        let center = pos.as_usize();
        let x = center % width;
        let mut count = 0;
        if x > 0 {
            count += u8::from(self.cell_is_free(CellIndex::from_usize(center - 1)));
        }
        if x + 1 < width {
            count += u8::from(self.cell_is_free(CellIndex::from_usize(center + 1)));
        }
        if center >= width {
            count += u8::from(self.cell_is_free(CellIndex::from_usize(center - width)));
        }
        if center + width < width * height {
            count += u8::from(self.cell_is_free(CellIndex::from_usize(center + width)));
        }
        count
    }

    /// Manhattan distance between two cells, straight from their cell indices: one divide and one
    /// multiply per cell, no `Position` built. Hot paths that compare many cells should use this
    /// rather than converting to positions first.
    #[inline]
    pub fn cell_distance(&self, a: CellIndex<T>, b: CellIndex<T>) -> usize {
        let width = usize::from(self.embedded.get_actual_width());
        let (a, b) = (a.as_usize(), b.as_usize());
        let (a_row, a_column) = (a / width, a % width);
        let (b_row, b_column) = (b / width, b % width);
        a_row.abs_diff(b_row) + a_column.abs_diff(b_column)
    }

    /// True when the two cells are one step apart: up, down, left, or right.
    ///
    /// For valid board cells, vertical neighbours are exactly `width` apart. Cells one apart
    /// are only neighbours when the lower one is not in the last column, which one modulo
    /// answers without decomposing both cells into rows and columns.
    #[inline]
    pub fn cells_are_adjacent(&self, a: CellIndex<T>, b: CellIndex<T>) -> bool {
        let width = usize::from(self.embedded.get_actual_width());
        let (a, b) = (a.as_usize(), b.as_usize());
        let difference = a.abs_diff(b);
        if difference == width {
            return true;
        }
        if difference != 1 {
            return false;
        }
        a.min(b) % width != width - 1
    }

    /// True when the two cells are at most `radius` steps apart. `radius == 1` is
    /// adjacency or equality; [`Self::cells_are_adjacent`] alone excludes equality.
    #[inline]
    pub fn cells_are_within(&self, a: CellIndex<T>, b: CellIndex<T>, radius: usize) -> bool {
        if radius == 1 {
            a == b || self.cells_are_adjacent(a, b)
        } else {
            self.cell_distance(a, b) <= radius
        }
    }

    /// True when some living snake other than `snake` has its head within `radius` steps of
    /// `cell`. One pass over the heads; a contest for a shared cell next turn is
    /// [`CONTEST_RADIUS`].
    pub fn other_head_within(&self, snake: SnakeId, cell: CellIndex<T>, radius: usize) -> bool {
        if radius == 1 {
            return self.all_heads_with_ids().any(|(id, head)| {
                id != snake && (cell == head || self.cells_are_adjacent(cell, head))
            });
        }
        let width = usize::from(self.embedded.get_actual_width());
        // Decompose the query cell once instead of once per head: the divide is not hoisted out of
        // the scan on its own, and doing it up front measured 27-33% faster than asking
        // `cells_are_within` per head, which re-divides the same cell every time.
        let cell = cell.as_usize();
        let (cell_row, cell_column) = (cell / width, cell % width);
        self.all_heads_with_ids().any(|(id, head)| {
            if id == snake {
                return false;
            }
            let head = head.as_usize();
            let (head_row, head_column) = (head / width, head % width);
            cell_row.abs_diff(head_row) + cell_column.abs_diff(head_column) <= radius
        })
    }

    /// Bit `Move::as_index()` is set for every move that does not immediately kill the snake at
    /// `head`. Computed straight from cell indices, so callers that only need the mask never
    /// build positions.
    #[inline]
    pub fn reasonable_move_mask(&self, head: CellIndex<T>) -> u8 {
        let width = usize::from(self.embedded.get_actual_width());
        let height = usize::from(self.embedded.get_actual_height());
        let center = head.as_usize();
        let x = center % width;
        let mut mask = 0u8;
        if x > 0 && self.cell_is_free(CellIndex::from_usize(center - 1)) {
            mask |= 1 << Move::Left.as_index();
        }
        if x + 1 < width && self.cell_is_free(CellIndex::from_usize(center + 1)) {
            mask |= 1 << Move::Right.as_index();
        }
        if center >= width && self.cell_is_free(CellIndex::from_usize(center - width)) {
            mask |= 1 << Move::Down.as_index();
        }
        if center + width < width * height
            && self.cell_is_free(CellIndex::from_usize(center + width))
        {
            mask |= 1 << Move::Up.as_index();
        }
        mask
    }

    /// Everything one candidate move of `snake` needs, gathered with a single read of the
    /// destination cell instead of one read per predicate.
    pub fn describe_move(&self, snake: SnakeId, head: CellIndex<T>, mv: Move) -> MoveTarget<T> {
        let width = usize::from(self.embedded.get_actual_width());
        let height = usize::from(self.embedded.get_actual_height());
        let center = head.as_usize();
        let x = center % width;
        let index = match mv {
            Move::Left if x > 0 => center - 1,
            Move::Right if x + 1 < width => center + 1,
            Move::Down if center >= width => center - width,
            Move::Up if center + width < width * height => center + width,
            // Off-board: the simulator's conventional fallback is still a direction, so report
            // it without letting the caller inspect a cell that does not exist.
            _ => return MoveTarget::off_board(),
        };

        let destination = CellIndex::from_usize(index);
        let cell = self.embedded.get_cell(destination);
        MoveTarget {
            destination,
            on_board: true,
            is_food: cell.is_food(),
            is_hazard: cell.is_hazard(),
            is_own_neck: cell.get_snake_id() == Some(snake) && cell.get_next_index() == Some(head),
        }
    }

    /// Heuristic for rollout depth, considers the following:
    /// - Whether we have less than two reasonable moves
    /// - Whether a nearby rival head could land on one of the same cells we could
    ///
    /// Both snakes move at once, so a conflict is a shared destination rather than a shared
    /// starting cell: each mask's destinations are compared, and the nearby filter cannot hide a
    /// collision because two heads one step from the same cell are within `CONTEST_RADIUS`.
    /// `tactical_conflict_matches_a_destination_collision_reference` pins that against a
    /// position-based reference.
    pub fn has_unresolved_tactical_conflict(&self, you: SnakeId) -> bool {
        let head_index = self.get_head_as_native_position(&you);
        let our_moves = self.reasonable_move_mask(head_index);
        // A mask is a set of move bits, so "fewer than two moves" is a bit count, not a
        // comparison against 2: a mask of 4 is two moves (up and left), not four.
        if our_moves.count_ones() < 2 {
            return true;
        }
        let width = usize::from(self.embedded.get_actual_width()) as isize;
        let head_base = head_index.as_usize() as isize;
        // Only a rival head within CONTEST_RADIUS can reach a cell we also reach: a shared
        // destination means each head is at most one step from it, so the filter costs nothing in
        // recall and saves computing a rival's mask for far away heads.
        self.all_heads_with_ids().any(|(id, their_head)| {
            if id == you || !self.cells_are_within(head_index, their_head, CONTEST_RADIUS) {
                return false;
            }
            let their_moves = self.reasonable_move_mask(their_head);
            let their_base = their_head.as_usize() as isize;
            // Destinations are compared as index arithmetic rather than collected into arrays:
            // both masks only hold on-board moves, so stepping an index cannot wrap a row, and
            // building the destination lists cost more than the comparison itself.
            Move::all_iter().any(|ours| {
                if our_moves & (1 << ours.as_index()) == 0 {
                    return false;
                }
                let ours = head_base + move_step(ours, width);
                Move::all_iter().any(|theirs| {
                    their_moves & (1 << theirs.as_index()) != 0
                        && ours == their_base + move_step(theirs, width)
                })
            })
        })
    }
}

/// One candidate destination, with the flags a move policy needs. Reading them together keeps
/// the search hot path down to one cell read per candidate instead of one per predicate.
#[derive(Clone, Copy, Debug)]
pub struct MoveTarget<T: CN> {
    /// Only meaningful when `on_board` is set.
    pub destination: CellIndex<T>,
    /// False for a move that leaves the board, in which case every other field is meaningless.
    pub on_board: bool,
    /// The destination holds food this turn.
    pub is_food: bool,
    /// The destination is a hazard this turn.
    pub is_hazard: bool,
    /// The destination is this snake's neck, so moving there reverses into our own body.
    pub is_own_neck: bool,
}

impl<T: CN> MoveTarget<T> {
    fn off_board() -> Self {
        Self {
            destination: CellIndex::from_i32(0),
            on_board: false,
            is_food: false,
            is_hazard: false,
            is_own_neck: false,
        }
    }
}

/// How far a move steps a cell index on a board `width` cells wide.
///
/// Only valid for a move from `reasonable_move_mask`, which never sets a bit for a move that leaves
/// the board, so stepping an index cannot wrap into another row. That is what makes this cheaper
/// than building a `Position` per move.
#[inline]
fn move_step(mv: Move, width: isize) -> isize {
    match mv {
        Move::Left => -1,
        Move::Right => 1,
        Move::Down => -width,
        Move::Up => width,
    }
}

/// Build a `MoveArray` from a mask produced by `reasonable_move_mask`, preserving `Move::all()`
/// order. An empty mask falls back to `Move::Up`, matching the previous iterator-based scan.
pub fn moves_from_mask(mask: u8) -> MoveArray {
    let mut moves: MoveArray = MoveArray::new();
    for mv in Move::all() {
        if mask & (1 << mv.as_index()) != 0 {
            moves.push(mv);
        }
    }
    if moves.is_empty() {
        moves.push(Move::Up);
    }
    moves
}

/// Uniformly chooses one of the legal moves encoded in `legal_mask`, where bit
/// `Move::as_index()` is set for each legal move.
///
/// An empty mask falls back to `Move::Up`, exactly matching the fallback used by
/// `reasonable_moves_for_each_snake`.
fn choose_from_legal_mask(legal_mask: u8, rng: &mut impl Rng) -> Move {
    let legal_count = legal_mask.count_ones();
    if legal_count == 0 {
        return Move::Up;
    }

    let mut remaining = legal_mask;
    let mut pick = rng.random_range(0..legal_count);
    while pick > 0 {
        // Clear the lowest set bit so we can find the `pick`-th legal move.
        remaining &= remaining - 1;
        pick -= 1;
    }

    Move::from_index(remaining.trailing_zeros() as usize)
}

impl<T: CN, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>
    CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>
{
    /// The native head cell of every living snake, in `SnakeId` order. Reads the head array
    /// directly instead of asking for one snake at a time, so the whole set costs a single pass
    /// over `MAX_SNAKES` slots. Dead snakes are skipped, matching the per-snake health check.
    ///
    /// ```
    /// # use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
    /// # fn check(board: &CellBoard4Snakes11x11) {
    /// for head in board.all_heads() {
    ///     // heads are already cell indices, so no position arithmetic is needed
    ///     println!("{}", board.cell_is_free(head));
    /// }
    /// # }
    /// ```
    pub fn all_heads(&self) -> impl Iterator<Item = CellIndex<T>> + '_ {
        self.all_heads_with_ids().map(|(_, head)| head)
    }

    /// `all_heads` together with the snake that owns each head, in `SnakeId` order. For callers
    /// that would otherwise look a head up again by id.
    pub fn all_heads_with_ids(&self) -> impl Iterator<Item = (SnakeId, CellIndex<T>)> + '_ {
        self.embedded.iter_living_heads()
    }

    /// The reasonable-move mask of every living snake, without building a `MoveArray` per
    /// snake. Hot paths that only test or iterate moves should prefer this. A snake with no
    /// reasonable move reports the same `Move::Up` fallback the list-based API uses.
    pub fn reasonable_move_masks(&self) -> ArrayVec<(SnakeId, u8), MAX_SNAKES> {
        self.embedded
            .iter_healths()
            .enumerate()
            .filter(|(_, health)| **health > 0)
            .map(|(index, _)| {
                let id = SnakeId(index as u8);
                let mask =
                    self.reasonable_move_mask(self.embedded.get_head_as_native_position(&id));
                let mask = if mask == 0 {
                    1 << Move::Up.as_index()
                } else {
                    mask
                };
                (id, mask)
            })
            .collect()
    }
}

impl<T: CN, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>
    RandomReasonableMovesGame for CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>
{
    fn random_reasonable_move_for_each_snake<'a>(
        &'a self,
        rng: &'a mut impl Rng,
    ) -> impl std::iter::Iterator<Item = (SnakeId, Move)> + 'a {
        self.all_heads_with_ids().map(move |(sid, head)| {
            let mask = self.reasonable_move_mask(head);
            (sid, choose_from_legal_mask(mask, rng))
        })
    }
}

impl<T: CN, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize> ReasonableMovesGame
    for CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>
{
    type SnakeMoves = ArrayVec<(SnakeId, MoveArray), MAX_SNAKES>;

    fn reasonable_moves_for_each_snake(&self) -> Self::SnakeMoves {
        self.all_heads_with_ids()
            .map(move |(sid, head)| (sid, moves_from_mask(self.reasonable_move_mask(head))))
            .collect()
    }
}

impl<D: Dimensions, N: CN, const BOARD_SIZE: usize, const MAX_SNAKES: usize>
    SimulableGame<MAX_SNAKES> for CellBoard<N, D, BOARD_SIZE, MAX_SNAKES>
{
    #[allow(clippy::type_complexity)]
    #[instrument(level = "trace", skip_all)]
    fn simulate_with_moves<S>(
        &self,
        snake_ids_and_moves: &[(Self::SnakeIDType, S)],
    ) -> Box<dyn Iterator<Item = (Action<MAX_SNAKES>, Self)> + '_>
    where
        S: Borrow<[Move]>,
    {
        Box::new(
            simulate_with_moves(&self.embedded, snake_ids_and_moves, EvaluateMode::Standard).map(
                |v| {
                    let (action, board) = v;
                    (action, Self { embedded: board })
                },
            ),
        )
    }
}

impl<T: CN, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>
    NeighborDeterminableGame for CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>
{
    fn possible_moves<'a>(
        &'a self,
        pos: &Self::NativePositionType,
    ) -> Box<dyn std::iter::Iterator<Item = (Move, CellIndex<T>)> + 'a> {
        let width = self.embedded.get_actual_width();
        let head_pos = pos.into_position(width);

        Box::new(
            Move::all_iter()
                .map(move |mv| {
                    let new_head = head_pos.add_vec(mv.to_vector());
                    let ci = CellIndex::new(new_head, width);

                    (mv, new_head, ci)
                })
                .filter(move |(_mv, new_head, _)| !self.off_board(*new_head))
                .map(|(mv, _, ci)| (mv, ci)),
        )
    }

    fn neighbors<'a>(
        &'a self,
        pos: &Self::NativePositionType,
    ) -> Box<dyn Iterator<Item = CellIndex<T>> + 'a> {
        let width = self.embedded.get_actual_width();
        let head_pos = pos.into_position(width);

        Box::new(
            Move::all_iter()
                .map(move |mv| {
                    let new_head = head_pos.add_vec(mv.to_vector());
                    let ci = CellIndex::new(new_head, width);

                    (new_head, ci)
                })
                .filter(move |(new_head, _)| !self.off_board(*new_head))
                .map(|(_, ci)| ci),
        )
    }
}

/// Enum that holds a Cell Board sized right for the given game
#[derive(Debug)]
pub enum BestCellBoard {
    /// A game that can have a max height and width of 7x7 and 4 snakes
    Tiny(Box<CellBoard4Snakes7x7>),
    /// A exactly 7x7 board with 4 snakes
    SmallExact(Box<CellBoard<u8, Fixed<7, 7>, { 7 * 7 }, 4>>),
    /// A game that can have a max height and width of 11x11 and 4 snakes
    Standard(Box<CellBoard4Snakes11x11>),
    /// A exactly 11x11 board with 4 snakes
    MediumExact(Box<CellBoard<u8, Fixed<11, 11>, { 11 * 11 }, 4>>),
    /// A game that can have a max height and width of 15x15 and 4 snakes
    LargestU8(Box<CellBoard8Snakes15x15>),
    /// A exactly 19x19 board with 4 snakes
    LargeExact(Box<CellBoard<u16, Fixed<19, 19>, { 19 * 19 }, 4>>),
    /// A board that fits the Arcade Maze map
    ArcadeMaze(Box<CellBoard<u16, ArcadeMaze, { 19 * 21 }, 4>>),
    /// A board that fits the Arcade Maze map
    ArcadeMaze8Snake(Box<CellBoard<u16, ArcadeMaze, { 19 * 21 }, 8>>),
    /// A game that can have a max height and width of 25x25 and 8 snakes
    Large(Box<CellBoard8Snakes25x25>),
    /// A game that can have a max height and width of 50x50 and 16 snakes
    Silly(Box<CellBoard16Snakes50x50>),
}

/// Trait to get the best sized cellboard for the given game. It returns the smallest Compact board
/// that has enough room to fit the given Wire game. If the game can't fit in any of our Compact
/// boards we panic. However the largest board available is MUCH larger than the biggest selectable
/// board in the Battlesnake UI
pub trait ToBestCellBoard {
    #[allow(missing_docs)]
    fn to_best_cell_board(self) -> Result<BestCellBoard, Box<dyn Error>>;
}

impl ToBestCellBoard for Game {
    fn to_best_cell_board(self) -> Result<BestCellBoard, Box<dyn Error>> {
        let width = self.board.width;
        let height = self.board.height;
        let num_snakes = self.board.snakes.len();
        let id_map = build_snake_id_map(&self);

        let best_board = if width == 7 && height == 7 && num_snakes <= 4 {
            BestCellBoard::SmallExact(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width <= 7 && height <= 7 && num_snakes <= 4 {
            BestCellBoard::Tiny(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width == 11 && height == 11 && num_snakes <= 4 {
            BestCellBoard::MediumExact(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width <= 11 && height <= 11 && num_snakes <= 4 {
            BestCellBoard::Standard(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width <= 15 && height <= 15 && num_snakes <= 8 {
            BestCellBoard::LargestU8(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width == 19 && height == 19 && num_snakes <= 4 {
            BestCellBoard::LargeExact(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width == 19 && height == 21 && num_snakes <= 4 {
            BestCellBoard::ArcadeMaze(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width == 19 && height == 21 && num_snakes <= 8 {
            BestCellBoard::ArcadeMaze8Snake(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width <= 25 && height < 25 && num_snakes <= 8 {
            BestCellBoard::Large(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width <= 50 && height <= 50 && num_snakes <= 16 {
            BestCellBoard::Silly(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else {
            panic!("No board was big enough")
        };

        Ok(best_board)
    }
}

#[cfg(test)]
mod test {

    use std::collections::{BTreeSet, HashMap};

    use itertools::Itertools;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    use super::*;
    use crate::{
        compact_representation::core::Cell, game_fixture, types::build_snake_id_map,
        wire_representation::Game as DEGame,
    };

    #[test]
    fn test_compact_board_conversion() {
        let start_of_game_fixture =
            game_fixture(include_str!("../../../fixtures/start_of_game.json"));
        let converted = Game::to_best_cell_board(start_of_game_fixture);
        assert!(converted.is_ok());
        let u = converted.unwrap();
        match u {
            BestCellBoard::MediumExact(_) => {}
            _ => panic!("expected standard board"),
        }

        let tiny_board = game_fixture(include_str!("../../../fixtures/7x7board.json"));
        let converted = Game::to_best_cell_board(tiny_board);
        assert!(converted.is_ok());
        let u = converted.unwrap();
        match u {
            BestCellBoard::SmallExact(_) => {}
            _ => panic!("expected standard board"),
        }

        let non_standard_small_board =
            game_fixture(include_str!("../../../fixtures/8x8board.json"));
        let converted = Game::to_best_cell_board(non_standard_small_board);
        assert!(converted.is_ok());
        let u = converted.unwrap();
        match u {
            BestCellBoard::Standard(_) => {}
            _ => panic!("expected standard board"),
        }
    }

    #[test]
    fn test_head_gettable() {
        let game_fixture = include_str!("../../../fixtures/late_stage.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
        assert_eq!(
            compact.get_head_as_position(&SnakeId(0)),
            Position { x: 4, y: 6 }
        );
        assert_eq!(
            compact.get_head_as_native_position(&SnakeId(0)),
            CellIndex(6 * 11 + 4)
        );
    }

    #[test]
    fn free_neighbor_count_matches_iterator_on_every_cell() {
        for fixture in [
            include_str!("../../../fixtures/start_of_game.json"),
            include_str!("../../../fixtures/late_stage.json"),
            include_str!("../../../fixtures/tail_chase.json"),
        ] {
            let game: DEGame = serde_json::from_str(fixture).unwrap();
            let cells = (game.board.width * game.board.height) as usize;
            let ids = build_snake_id_map(&game);
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
            for index in 0..cells {
                let pos = CellIndex::from_usize(index);
                assert_eq!(
                    usize::from(board.free_neighbor_count(pos)),
                    board.free_neighbors(pos).count(),
                    "cell {index}"
                );
            }
        }
    }

    #[test]
    fn all_heads_agrees_with_the_per_snake_reads() {
        for fixture in [
            include_str!("../../../fixtures/start_of_game.json"),
            include_str!("../../../fixtures/late_stage.json"),
            include_str!("../../../fixtures/tail_chase.json"),
        ] {
            let game: DEGame = serde_json::from_str(fixture).unwrap();
            let ids = build_snake_id_map(&game);
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();

            let living: Vec<SnakeId> = board
                .all_heads_with_ids()
                .map(|(id, _)| id)
                .filter(|id| board.get_health_i64(id) > 0)
                .collect();
            assert!(!living.is_empty(), "{fixture} should have live snakes");

            let per_snake: Vec<CellIndex<u8>> = (0..4)
                .map(SnakeId)
                .filter(|id| board.get_health_i64(id) > 0)
                .map(|id| board.get_head_as_native_position(&id))
                .collect();
            assert_eq!(board.all_heads().collect::<Vec<_>>(), per_snake);
            assert_eq!(
                board
                    .all_heads_with_ids()
                    .map(|(_, head)| head)
                    .collect::<Vec<_>>(),
                per_snake
            );
            // ids ascend and agree with the head looked up for that id
            assert!(living.windows(2).all(|w| w[0].0 < w[1].0));
            for (id, head) in board.all_heads_with_ids() {
                assert_eq!(head, board.get_head_as_native_position(&id));
            }
            assert_eq!(living.len(), board.all_heads().count());
        }
    }

    #[test]
    fn adjacency_and_radius_match_geometry_across_dimensions() {
        // Include one-column boards, row seams, rectangular boards, and u16 indices.
        for (width, height) in [(1, 7), (7, 1), (2, 3), (7, 7), (11, 11), (19, 21), (25, 25)] {
            let mut game: DEGame =
                serde_json::from_str(include_str!("../../../fixtures/start_of_game.json")).unwrap();
            game.board.width = width;
            game.board.height = height;
            game.board.snakes.clear();
            game.board.food.clear();
            game.board.hazards.clear();
            let ids = build_snake_id_map(&game);
            let board: CellBoard<u16, Custom, 625, 4> = game.as_cell_board(&ids).unwrap();
            for a in 0..(width * height) as usize {
                for b in 0..(width * height) as usize {
                    let width = width as usize;
                    let distance =
                        (a / width).abs_diff(b / width) + (a % width).abs_diff(b % width);
                    let (a, b) = (CellIndex::from_usize(a), CellIndex::from_usize(b));
                    assert_eq!(
                        board.cells_are_adjacent(a, b),
                        distance == 1,
                        "{width}x{height}: {a:?} {b:?}"
                    );
                    for radius in [0, 1, 2, 3] {
                        assert_eq!(
                            board.cells_are_within(a, b, radius),
                            distance <= radius,
                            "{width}x{height}, radius {radius}: {a:?} {b:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn cell_distance_matches_positions_on_every_pair() {
        for fixture in [
            include_str!("../../../fixtures/start_of_game.json"),
            include_str!("../../../fixtures/late_stage.json"),
            include_str!("../../../fixtures/tail_chase.json"),
        ] {
            let game: DEGame = serde_json::from_str(fixture).unwrap();
            let cells = (game.board.width * game.board.height) as usize;
            let ids = build_snake_id_map(&game);
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
            let width = game.board.width as usize;
            let manhattan = |a: usize, b: usize| {
                let (a_x, a_y) = (a % width, a / width);
                let (b_x, b_y) = (b % width, b / width);
                a_x.abs_diff(b_x) + a_y.abs_diff(b_y)
            };
            for a in 0..cells {
                for b in 0..cells {
                    let (a, b) = (CellIndex::from_usize(a), CellIndex::from_usize(b));
                    let expected = manhattan(a.as_usize(), b.as_usize());
                    assert_eq!(board.cell_distance(a, b), expected, "{a:?} {b:?}");
                    assert_eq!(
                        board.cells_are_adjacent(a, b),
                        expected == 1,
                        "adjacent {a:?} {b:?}"
                    );
                    assert_eq!(
                        board.cells_are_within(a, b, 2),
                        expected <= 2,
                        "{a:?} {b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn other_head_within_finds_every_contesting_head() {
        for fixture in [
            include_str!("../../../fixtures/start_of_game.json"),
            include_str!("../../../fixtures/late_stage.json"),
            include_str!("../../../fixtures/tail_chase.json"),
            include_str!("../../../fixtures/4_snake_game.json"),
        ] {
            let game: DEGame = serde_json::from_str(fixture).unwrap();
            let ids = build_snake_id_map(&game);
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
            for me in board.all_heads_with_ids().map(|(id, _)| id) {
                // Query every cell, including other heads (distance zero) and row seams.
                for cell in 0..(game.board.width * game.board.height) as usize {
                    let head = CellIndex::from_usize(cell);
                    for radius in [0, 1, 2, 3] {
                        let expected = board.all_heads_with_ids().any(|(id, other)| {
                            id != me && board.cell_distance(head, other) <= radius
                        });
                        assert_eq!(
                            board.other_head_within(me, head, radius),
                            expected,
                            "{me:?} at {head:?} within {radius}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn contest_radius_equals_sharing_a_cell_next_turn() {
        // Brute force the meaning of CONTEST_RADIUS: two heads contest when one orthogonal step
        // from each of them lands on the same cell. Positions, not the index helpers, so this
        // does not just restate the implementation.
        for fixture in [
            include_str!("../../../fixtures/start_of_game.json"),
            include_str!("../../../fixtures/late_stage.json"),
        ] {
            let game: DEGame = serde_json::from_str(fixture).unwrap();
            let width = game.board.width as i32;
            let height = game.board.height as i32;
            let ids = build_snake_id_map(&game);
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
            // A head can contest its own cell (the opponent moving onto it) as well as any cell
            // one step away, so the cells a snake can claim include where it stands.
            let claims = |pos: Position| {
                std::iter::once(pos)
                    .chain(Move::all_iter().map(|mv| pos.add_vec(mv.to_vector())))
                    .filter(|next| next.x >= 0 && next.x < width && next.y >= 0 && next.y < height)
                    .collect::<Vec<_>>()
            };
            let cells = (game.board.width * game.board.height) as usize;
            for a in 0..cells {
                let a_position = Position {
                    x: a as i32 % width,
                    y: a as i32 / width,
                };
                let a_claims = claims(a_position);
                for b in 0..cells {
                    if a == b {
                        continue;
                    }
                    let b_position = Position {
                        x: b as i32 % width,
                        y: b as i32 / width,
                    };
                    let shared = claims(b_position)
                        .iter()
                        .any(|cell| a_claims.contains(cell));
                    assert_eq!(
                        board.cells_are_within(
                            CellIndex::from_usize(a),
                            CellIndex::from_usize(b),
                            CONTEST_RADIUS
                        ),
                        shared,
                        "{a_position:?} and {b_position:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn tactical_conflict_matches_a_destination_collision_reference() {
        // Reference built from Positions and move vectors, so it shares no arithmetic with the
        // index-based implementation it checks.
        for fixture in [
            include_str!("../../../fixtures/start_of_game.json"),
            include_str!("../../../fixtures/late_stage.json"),
            include_str!("../../../fixtures/tail_chase.json"),
            include_str!("../../../fixtures/cornered.json"),
        ] {
            let game: DEGame = serde_json::from_str(fixture).unwrap();
            let ids = build_snake_id_map(&game);
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
            let reference = |you: SnakeId| {
                let our_moves = board.reasonable_move_masks();
                let (id, mask) = our_moves
                    .iter()
                    .find(|(id, _)| *id == you)
                    .expect("a living snake has a mask");
                if mask.count_ones() < 2 {
                    return true;
                }
                let ours: Vec<Position> = moves_from_mask(*mask)
                    .iter()
                    .map(|mv| board.get_head_as_position(id).add_vec(mv.to_vector()))
                    .collect();
                board.all_heads_with_ids().any(|(other, their_head)| {
                    if other == you {
                        return false;
                    }
                    let their_position = board.get_head_as_position(&other);
                    let distance = (board.get_head_as_position(id).x - their_position.x).abs()
                        + (board.get_head_as_position(id).y - their_position.y).abs();
                    if distance > CONTEST_RADIUS as i32 {
                        return false;
                    }
                    let their_mask = board.reasonable_move_mask(their_head);
                    moves_from_mask(their_mask).iter().any(|mv| {
                        let theirs = their_position.add_vec(mv.to_vector());
                        ours.contains(&theirs)
                    })
                })
            };
            for (id, _) in board.all_heads_with_ids() {
                assert_eq!(
                    board.has_unresolved_tactical_conflict(id),
                    reference(id),
                    "{fixture} {id:?}"
                );
            }
        }
    }

    #[test]
    fn move_masks_agree_with_the_move_list_api() {
        for fixture in [
            include_str!("../../../fixtures/start_of_game.json"),
            include_str!("../../../fixtures/late_stage.json"),
            include_str!("../../../fixtures/tail_chase.json"),
        ] {
            let game: DEGame = serde_json::from_str(fixture).unwrap();
            let ids = build_snake_id_map(&game);
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
            let listed = board.reasonable_moves_for_each_snake();
            for (id, mask) in board.reasonable_move_masks() {
                let moves = listed
                    .iter()
                    .find(|(listed_id, _)| *listed_id == id)
                    .map(|(_, moves)| *moves)
                    .expect("every masked snake appears in the move list");
                assert_eq!(moves_from_mask(mask).as_slice(), moves.as_slice());
            }
            assert_eq!(listed.len(), board.reasonable_move_masks().len());
        }
    }

    #[test]
    fn describe_move_agrees_with_the_per_predicate_reads() {
        for fixture in [
            include_str!("../../../fixtures/start_of_game.json"),
            include_str!("../../../fixtures/late_stage.json"),
        ] {
            let game: DEGame = serde_json::from_str(fixture).unwrap();
            let ids = build_snake_id_map(&game);
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).unwrap();
            for (id, _) in board.reasonable_move_masks() {
                let head = board.get_head_as_native_position(&id);
                let head_pos = board.get_head_as_position(&id);
                for mv in Move::all() {
                    let target = board.describe_move(id, head, mv);
                    let destination = head_pos.add_vec(mv.to_vector());
                    let off_board = board.off_board(destination);
                    assert_eq!(target.on_board, !off_board, "{mv} {:?}", id);
                    if off_board {
                        continue;
                    }
                    let native = board.native_from_position(destination);
                    assert_eq!(target.destination, native);
                    assert_eq!(target.is_food, board.is_food(&native));
                    assert_eq!(target.is_hazard, board.is_hazard(&native));
                    assert_eq!(target.is_own_neck, board.is_neck(&id, &native));
                }
            }
        }
    }

    #[test]
    fn test_tail_collision() {
        let game_fixture = include_str!("../../../fixtures/start_of_game.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let mut compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
        let moves = [
            Move::Left,
            Move::Down,
            Move::Right,
            Move::Up,
            Move::Left,
            Move::Down,
        ];
        eprintln!("{}", compact);
        for mv in moves {
            let res = compact
                .simulate_with_moves(&[(SnakeId(0), [mv].as_slice())])
                .collect_vec();
            compact = res[0].1;
            eprintln!("{}", compact);
        }
        assert!(compact.get_health(&SnakeId(0)) > 0);
    }

    #[test]
    fn single_action_matches_general_simulator() {
        let fixtures = [
            (
                "start_of_game",
                include_str!("../../../fixtures/start_of_game.json"),
            ),
            (
                "head_collision",
                include_str!("../../../fixtures/tree_search_collision.json"),
            ),
            (
                "body_collision",
                include_str!("../../../fixtures/body_collision.json"),
            ),
            (
                "forced_death",
                include_str!("../../../fixtures/all-options-dead-prefer-out-of-bounds.json"),
            ),
            ("cornered", include_str!("../../../fixtures/cornered.json")),
        ];

        for (name, fixture) in fixtures {
            let game: DEGame = serde_json::from_str(fixture).expect("valid fixture");
            let ids = build_snake_id_map(&game);
            let snake_count = game.board.snakes.len();
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).expect("valid board");

            for moves in (0..snake_count)
                .map(|index| {
                    Move::all()
                        .into_iter()
                        .map(move |mv| (SnakeId(index as u8), mv))
                })
                .multi_cartesian_product()
            {
                let selections: Vec<_> = moves.iter().map(|(id, mv)| (*id, [*mv])).collect();
                let expected = board
                    .simulate_with_moves(&selections)
                    .next()
                    .expect("one joint action");
                let actual = board.simulate_single_action(&moves);
                assert_eq!(actual, expected, "fixture {name}, moves {moves:?}");
            }
        }
    }

    #[test]
    fn single_action_collision_and_forced_death_outcomes() {
        let game: DEGame =
            serde_json::from_str(include_str!("../../../fixtures/tree_search_collision.json"))
                .expect("valid fixture");
        let ids = build_snake_id_map(&game);
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).expect("valid board");
        let (_, next) =
            board.simulate_single_action(&[(SnakeId(0), Move::Right), (SnakeId(1), Move::Up)]);
        assert_eq!(next.get_health(&SnakeId(0)), 0);
        assert!(next.get_health(&SnakeId(1)) > 0);

        let game: DEGame = serde_json::from_str(include_str!("../../../fixtures/cornered.json"))
            .expect("valid fixture");
        let ids = build_snake_id_map(&game);
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).expect("valid board");
        let (_, next) = board.simulate_single_action(&[(SnakeId(0), Move::Up)]);
        assert_eq!(next.get_health(&SnakeId(0)), 0);
    }

    #[test]
    fn single_action_reverse_into_neck_is_fatal() {
        let game: DEGame =
            serde_json::from_str(include_str!("../../../fixtures/start_of_game.json"))
                .expect("valid fixture");
        let ids = build_snake_id_map(&game);
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).expect("valid board");
        let (_, reversed) = board.simulate_single_action(&[(SnakeId(0), Move::Up)]);
        assert_eq!(reversed.get_health(&SnakeId(0)), 0);
        let (_, forward) = board.simulate_single_action(&[(SnakeId(0), Move::Down)]);
        assert!(forward.get_health(&SnakeId(0)) > 0);
    }

    #[test]
    fn test_set_hazard() {
        let mut c: Cell<u8> = Cell::empty();
        c.set_food();
        assert!(c.is_food());
        c.set_hazard();
        assert!(c.is_food());
        assert!(c.is_hazard());
        assert!(!c.is_head());
        assert!(!c.is_body());
    }

    #[test]
    fn test_clear_hazard() {
        let mut c: Cell<u8> = Cell::empty();
        c.set_food();
        assert!(c.is_food());
        c.set_hazard();
        c.clear_hazard();
        assert!(c.is_food());
        assert!(!c.is_hazard());
        assert!(!c.is_head());
        assert!(!c.is_body());
        let mut c: Cell<u8> = Cell::make_double_stacked_piece(SnakeId(0), CellIndex(0));
        c.set_hazard();
        c.clear_hazard();
        assert!(c.is_body());
        assert!(!c.is_hazard());
    }

    #[test]
    fn test_remove() {
        let mut c: Cell<u8> = Cell::make_body_piece(SnakeId(3), CellIndex(17));
        c.remove();
        c.set_hazard();
        assert!(c.is_empty());
        assert!(c.is_hazard());
        assert!(c.get_snake_id().is_none());
        assert!(c.get_idx() == CellIndex(0));
    }
    #[test]
    fn test_set_food() {
        let mut c: Cell<u8> = Cell::empty();
        c.set_food();
        c.set_hazard();
        assert!(c.is_food());
        assert!(c.is_hazard());
        assert!(c.get_snake_id().is_none());
        assert!(c.get_idx() == CellIndex(0));
    }

    #[test]
    fn test_set_head() {
        let mut c: Cell<u8> = Cell::empty();
        c.set_head(SnakeId(3), CellIndex(17));
        c.set_hazard();
        assert!(c.is_head());
        assert!(c.is_hazard());
        assert!(c.get_snake_id().unwrap() == SnakeId(3));
        assert!(c.get_idx() == CellIndex(17));
    }

    #[test]
    fn test_food_queryable() {
        let game_fixture = include_str!("../../../fixtures/late_stage.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        assert!(!compact.is_food(&CellIndex(6 * 11 + 4)));

        assert!(compact.is_food(&CellIndex(2 * 11)));
        assert!(compact.is_food(&CellIndex(9 * 11)));
        assert!(compact.is_food(&CellIndex(3 * 11 + 4)));
    }

    #[test]
    fn test_neighbors_and_possible_moves_start_of_game() {
        let game_fixture = include_str!("../../../fixtures/start_of_game.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        let head = compact.get_head_as_native_position(&SnakeId(0));
        assert_eq!(head, CellIndex(8 * 11 + 5));

        let expected_possible_moves = vec![
            (Move::Up, CellIndex(9 * 11 + 5)),
            (Move::Down, CellIndex(7 * 11 + 5)),
            (Move::Left, CellIndex(8 * 11 + 4)),
            (Move::Right, CellIndex(8 * 11 + 6)),
        ];

        assert_eq!(
            compact.possible_moves(&head).collect::<Vec<_>>(),
            expected_possible_moves
        );

        assert_eq!(
            compact.neighbors(&head).collect::<Vec<_>>(),
            expected_possible_moves
                .into_iter()
                .map(|(_, pos)| pos)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_neighbors_and_possible_moves_cornered() {
        let game_fixture = include_str!("../../../fixtures/cornered.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        let head = compact.get_head_as_native_position(&SnakeId(0));
        assert_eq!(head, CellIndex(10 * 11));

        let expected_possible_moves = vec![
            (Move::Down, CellIndex(9 * 11)),
            (Move::Right, CellIndex(10 * 11 + 1)),
        ];

        assert_eq!(
            compact.possible_moves(&head).collect::<Vec<_>>(),
            expected_possible_moves
        );

        assert_eq!(
            compact.neighbors(&head).collect::<Vec<_>>(),
            expected_possible_moves
                .into_iter()
                .map(|(_, pos)| pos)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_tail_chase() {
        let game_fixture = include_str!("../../../fixtures/tail_chase.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        let head = compact.get_head_as_native_position(&SnakeId(0));
        assert_eq!(head, CellIndex(0));

        let reasonable_moves = compact.reasonable_moves_for_each_snake();
        assert_eq!(reasonable_moves.capacity(), 4);
        let reasonable_moves_for_me = reasonable_moves[0].1;

        assert_eq!(reasonable_moves_for_me.as_slice(), &[Move::Up]);
    }

    /// Recomputes the legality bitmask for one snake, mirroring the predicate used by
    /// `reasonable_moves_for_each_snake`. Used to prove a fixture genuinely has no legal moves.
    fn legal_mask_for(board: &CellBoard4Snakes11x11, sid: SnakeId) -> u8 {
        let width = board.embedded.get_actual_width();
        let head_pos = board.get_head_as_position(&sid);
        let mut mask = 0u8;
        for mv in Move::all() {
            let new_head = head_pos.add_vec(mv.to_vector());
            let ci = CellIndex::new(new_head, width);
            if !board.off_board(new_head)
                && (!board.embedded.cell_is_body(ci) || board.embedded.cell_is_single_tail(ci))
                && !board.embedded.cell_is_snake_head(ci)
            {
                mask |= 1 << mv.as_index();
            }
        }
        mask
    }

    fn reasonable_support(board: &CellBoard4Snakes11x11) -> HashMap<SnakeId, BTreeSet<Move>> {
        board
            .reasonable_moves_for_each_snake()
            .into_iter()
            .map(|(sid, moves)| (sid, moves.into_iter().collect()))
            .collect()
    }

    #[test]
    fn random_move_support_matches_reasonable_moves() {
        let fixtures: &[(&str, &str)] = &[
            (
                "start_of_game",
                include_str!("../../../fixtures/start_of_game.json"),
            ),
            ("cornered", include_str!("../../../fixtures/cornered.json")),
            (
                "tail_chase",
                include_str!("../../../fixtures/tail_chase.json"),
            ),
            (
                "late_stage",
                include_str!("../../../fixtures/late_stage.json"),
            ),
            (
                "all_options_dead",
                include_str!("../../../fixtures/all-options-dead-prefer-out-of-bounds.json"),
            ),
        ];

        let mut rng = SmallRng::seed_from_u64(0x5EED_1234);

        for (name, fixture) in fixtures {
            let game: DEGame = serde_json::from_str(fixture).expect("valid fixture");
            let ids = build_snake_id_map(&game);
            let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).expect("valid board");

            let reasonable = reasonable_support(&board);

            let mut sampled: HashMap<SnakeId, BTreeSet<Move>> = HashMap::new();
            for _ in 0..1_000 {
                for (sid, mv) in board.random_reasonable_move_for_each_snake(&mut rng) {
                    let legal = reasonable
                        .get(&sid)
                        .unwrap_or_else(|| panic!("{name}: move for non-living snake {sid:?}"));
                    assert!(
                        legal.contains(&mv),
                        "{name}: sampled illegal move {mv:?} for {sid:?}"
                    );
                    sampled.entry(sid).or_default().insert(mv);
                }
            }

            assert_eq!(
                sampled, reasonable,
                "{name}: sampled move support differs from the legal support"
            );
        }
    }

    #[test]
    fn random_move_sampling_is_roughly_uniform_over_legal_moves() {
        let game: DEGame =
            serde_json::from_str(include_str!("../../../fixtures/start_of_game.json"))
                .expect("valid fixture");
        let ids = build_snake_id_map(&game);
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).expect("valid board");

        let reasonable = reasonable_support(&board);
        let legal = &reasonable[&SnakeId(0)];
        assert_eq!(
            legal.len(),
            3,
            "fixture is expected to have three legal moves"
        );

        const DRAWS: usize = 20_000;
        let mut counts: HashMap<Move, usize> = HashMap::new();
        let mut rng = SmallRng::seed_from_u64(0xC0FF_EE);
        for _ in 0..DRAWS {
            let sampled: HashMap<SnakeId, Move> = board
                .random_reasonable_move_for_each_snake(&mut rng)
                .collect();
            *counts.entry(sampled[&SnakeId(0)]).or_default() += 1;
        }

        // The sampler is uniform, so each move should land near DRAWS / legal.len().
        // The band is intentionally loose: its job is to catch gross bias (for example
        // always returning the first legal move) without being flaky.
        let expected = DRAWS as f64 / legal.len() as f64;
        for mv in legal {
            let count = counts.get(mv).copied().unwrap_or(0) as f64;
            assert!(
                (count - expected).abs() <= expected * 0.15,
                "move {mv:?} was sampled {count} times, expected about {expected}"
            );
        }
    }

    /// Snake 0's head sits in the bottom-left corner with its own body directly below it and
    /// another snake's head to its right, so every one of its four moves is illegal.
    const NO_LEGAL_MOVES_FIXTURE: &str = r##"{
        "game": {
            "id": "no-legal-moves",
            "ruleset": { "name": "standard", "version": "v.1.2.3" },
            "timeout": 500
        },
        "turn": 42,
        "you": {
            "health": 100,
            "id": "you",
            "name": "#22aa34",
            "body": [{ "x": 0, "y": 0 }, { "x": 0, "y": 1 }, { "x": 0, "y": 2 }],
            "head": { "x": 0, "y": 0 },
            "length": 3
        },
        "board": {
            "food": [],
            "hazards": [],
            "height": 11,
            "width": 11,
            "snakes": [
                {
                    "health": 100,
                    "id": "you",
                    "name": "#22aa34",
                    "body": [{ "x": 0, "y": 0 }, { "x": 0, "y": 1 }, { "x": 0, "y": 2 }],
                    "head": { "x": 0, "y": 0 },
                    "length": 3
                },
                {
                    "health": 100,
                    "id": "#FF8331",
                    "name": "#FF8331",
                    "body": [{ "x": 1, "y": 0 }, { "x": 2, "y": 0 }],
                    "head": { "x": 1, "y": 0 },
                    "length": 2
                }
            ]
        }
    }"##;

    #[test]
    fn random_move_falls_back_to_up_when_no_legal_moves() {
        let game: DEGame = serde_json::from_str(NO_LEGAL_MOVES_FIXTURE).expect("valid fixture");
        let ids = build_snake_id_map(&game);
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).expect("valid board");

        // Prove the fixture is interesting: snake 0 really has zero legal moves, so the `Up`
        // we observe below can only come from the fallback.
        assert_eq!(legal_mask_for(&board, SnakeId(0)), 0);

        let reasonable = reasonable_support(&board);
        assert_eq!(
            reasonable[&SnakeId(0)].iter().copied().collect::<Vec<_>>(),
            vec![Move::Up]
        );

        let mut rng = SmallRng::seed_from_u64(7);
        for _ in 0..100 {
            let sampled: HashMap<SnakeId, Move> = board
                .random_reasonable_move_for_each_snake(&mut rng)
                .collect();
            assert_eq!(sampled[&SnakeId(0)], Move::Up);
        }

        // The chooser itself always falls back on an empty mask.
        for _ in 0..100 {
            assert_eq!(choose_from_legal_mask(0, &mut rng), Move::Up);
        }
    }
}
