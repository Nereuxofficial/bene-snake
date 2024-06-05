use crate::{evaluate_board, Simulator};
use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::{SimulableGame, SnakeId};
use std::borrow::Cow;
use std::cell::Cell;

struct Node {
    eval: f32,
    state: CellBoard4Snakes11x11,
    children: Vec<Node>,
}

impl Node {
    fn new(state: CellBoard4Snakes11x11, you: &SnakeId, snake_ids: Cow<Vec<SnakeId>>) -> Node {
        Node {
            eval: evaluate_board(&state, you, snake_ids),
            children: Vec::with_capacity(16),
            state,
        }
    }

    fn get_size(&self) -> usize {
        self.children.len()
            + self
                .children
                .iter()
                .map(|child| child.get_size())
                .sum::<usize>()
    }

    fn add_child(&mut self, child: Node) {
        self.children.push(child);
    }
    /* TODO
        fn generate_depth_limited(
            initial_state: CellBoard4Snakes11x11,
            depth: usize,
            you: &SnakeId,
            snake_ids: Cow<Vec<SnakeId>>,
        ) -> Self {
            let mut root = Node::new(initial_state, you, snake_ids.clone());
            if depth == 0 {
                return root;
            }
            let mut children = root
                .state
                .simulate(&Simulator {}, snake_ids.to_vec())
                .collect();
        }
    */
    fn get_subtrees(&self) -> &Vec<Node> {
        &self.children
    }
    fn get_state(&self) -> &CellBoard4Snakes11x11 {
        &self.state
    }
    fn get_eval(&self) -> f32 {
        self.eval
    }
}
