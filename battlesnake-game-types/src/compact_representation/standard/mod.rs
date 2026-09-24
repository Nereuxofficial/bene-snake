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
    types::{Move, SimulableGame, SimulatorInstruments},
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
    pub fn simulate_single_action<I: SimulatorInstruments>(
        &self,
        instruments: &I,
        moves: &[(SnakeId, Move)],
    ) -> (Action<MAX_SNAKES>, Self) {
        let (action, embedded) =
            simulate_single_action(&self.embedded, instruments, moves, EvaluateMode::Standard);
        (action, Self { embedded })
    }
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
    RandomReasonableMovesGame for CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>
{
    fn random_reasonable_move_for_each_snake<'a>(
        &'a self,
        rng: &'a mut impl Rng,
    ) -> impl std::iter::Iterator<Item = (SnakeId, Move)> + 'a {
        let width = self.embedded.get_actual_width();
        self.embedded
            .iter_healths()
            .enumerate()
            .filter(|(_, health)| **health > 0)
            .map(move |(idx, _)| {
                let sid = SnakeId(idx as u8);
                let head_pos = self.get_head_as_position(&sid);

                // One scan of the four moves, recording legality in a stack-only bitmask.
                let mut legal_mask = 0u8;
                for mv in Move::all() {
                    let new_head = head_pos.add_vec(mv.to_vector());
                    let ci = CellIndex::new(new_head, width);
                    if !self.off_board(new_head)
                        && (!self.embedded.cell_is_body(ci)
                            || self.embedded.cell_is_single_tail(ci))
                        && !self.embedded.cell_is_snake_head(ci)
                    {
                        legal_mask |= 1 << mv.as_index();
                    }
                }

                (sid, choose_from_legal_mask(legal_mask, rng))
            })
    }
}

impl<T: CN, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize> ReasonableMovesGame
    for CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>
{
    type SnakeMoves = ArrayVec<(SnakeId, MoveArray), MAX_SNAKES>;

    fn reasonable_moves_for_each_snake(&self) -> Self::SnakeMoves {
        let width = self.embedded.get_actual_width();
        self.embedded
            .iter_healths()
            .enumerate()
            .filter(|(_, health)| **health > 0)
            .map(move |(idx, _)| {
                let head_pos = self.get_head_as_position(&SnakeId(idx as u8));

                let mut mvs: MoveArray = IntoIterator::into_iter(Move::all())
                    .filter(|mv| {
                        let new_head = head_pos.add_vec(mv.to_vector());
                        let ci = CellIndex::new(new_head, width);

                        !self.off_board(new_head)
                            && (!self.embedded.cell_is_body(ci)
                                || self.embedded.cell_is_single_tail(ci))
                            && !self.embedded.cell_is_snake_head(ci)
                    })
                    .collect();
                if mvs.is_empty() {
                    mvs.push(Move::Up);
                }

                (SnakeId(idx as u8), mvs)
            })
            .collect()
    }
}

impl<
    T: SimulatorInstruments,
    D: Dimensions,
    N: CN,
    const BOARD_SIZE: usize,
    const MAX_SNAKES: usize,
> SimulableGame<T, MAX_SNAKES> for CellBoard<N, D, BOARD_SIZE, MAX_SNAKES>
{
    #[allow(clippy::type_complexity)]
    #[instrument(level = "trace", skip_all)]
    fn simulate_with_moves<S>(
        &self,
        instruments: &T,
        snake_ids_and_moves: &[(Self::SnakeIDType, S)],
    ) -> Box<dyn Iterator<Item = (Action<MAX_SNAKES>, Self)> + '_>
    where
        S: Borrow<[Move]>,
    {
        Box::new(
            simulate_with_moves(
                &self.embedded,
                instruments,
                snake_ids_and_moves,
                EvaluateMode::Standard,
            )
            .map(|v| {
                let (action, board) = v;
                (action, Self { embedded: board })
            }),
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
    #[derive(Debug)]
    struct Instruments;
    impl SimulatorInstruments for Instruments {
        fn observe_simulation(&self, _: std::time::Duration) {}
    }

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
        let instruments = Instruments;
        eprintln!("{}", compact);
        for mv in moves {
            let res = compact
                .simulate_with_moves(&instruments, &[(SnakeId(0), [mv].as_slice())])
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
        let instruments = Instruments;

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
                    .simulate_with_moves(&instruments, &selections)
                    .next()
                    .expect("one joint action");
                let actual = board.simulate_single_action(&instruments, &moves);
                assert_eq!(actual, expected, "fixture {name}, moves {moves:?}");
            }
        }
    }

    #[test]
    fn single_action_collision_and_forced_death_outcomes() {
        let instruments = Instruments;
        let game: DEGame =
            serde_json::from_str(include_str!("../../../fixtures/tree_search_collision.json"))
                .expect("valid fixture");
        let ids = build_snake_id_map(&game);
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).expect("valid board");
        let (_, next) = board.simulate_single_action(
            &instruments,
            &[(SnakeId(0), Move::Right), (SnakeId(1), Move::Up)],
        );
        assert_eq!(next.get_health(&SnakeId(0)), 0);
        assert!(next.get_health(&SnakeId(1)) > 0);

        let game: DEGame = serde_json::from_str(include_str!("../../../fixtures/cornered.json"))
            .expect("valid fixture");
        let ids = build_snake_id_map(&game);
        let board: CellBoard4Snakes11x11 = game.as_cell_board(&ids).expect("valid board");
        let (_, next) = board.simulate_single_action(&instruments, &[(SnakeId(0), Move::Up)]);
        assert_eq!(next.get_health(&SnakeId(0)), 0);
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
