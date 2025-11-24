use crate::{evaluate_board, Simulator};
use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::{ReasonableMovesGame, SimulableGame, SnakeId};
use std::borrow::Cow;

struct Node {
    eval: u16,
    state: CellBoard4Snakes11x11,
    children: Vec<Node>,
}

impl Node {
    fn new(state: CellBoard4Snakes11x11, you: &SnakeId, snake_ids: Cow<[SnakeId]>) -> Node {
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

    fn generate_depth_limited(
        initial_state: CellBoard4Snakes11x11,
        depth: usize,
        you: &SnakeId,
        snake_ids: Cow<[SnakeId]>,
    ) -> Self {
        let mut reasonable_moves = initial_state.reasonable_moves_for_each_snake();
        let children = initial_state.simulate_with_moves(&Simulator {}, &mut reasonable_moves);
        let mut root = Node::new(initial_state, you, snake_ids.clone());
        if depth == 0 {
            return root;
        }
        for (action, state) in children {
            let mut child = Node::new(state, you, snake_ids.clone());
            child.add_child(Node::generate_depth_limited(
                state,
                depth - 1,
                you,
                snake_ids.clone(),
            ));
            root.add_child(child);
        }
        root
    }
}
