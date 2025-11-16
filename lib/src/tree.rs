use crate::{evaluate_board, Simulator};
use battlesnake_game_types::compact_representation::standard::CellBoard4Snakes11x11;
use battlesnake_game_types::types::{Action, SimulableGame, SnakeId};
use std::borrow::Cow;

struct Node {
    eval: u16,
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

    fn generate_depth_limited(
        initial_state: CellBoard4Snakes11x11,
        depth: usize,
        you: &SnakeId,
        snake_ids: Cow<Vec<SnakeId>>,
    ) -> Self {
        let mut children: Box<dyn Iterator<Item = (Action<4>, CellBoard4Snakes11x11)> + '_> =
            initial_state.simulate(&Simulator {}, &snake_ids);
        let mut root = Node::new(initial_state, you, snake_ids.clone());
        if depth == 0 {
            return root;
        }
        while let Some((action, state)) = children.next() {
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
