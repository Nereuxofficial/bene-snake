//! Exact finite-horizon existential/universal search and independently checked DAGs.
use crate::{
    rules::{self, State, Transition},
    schema::*,
};
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Branch {
    pub moves: BTreeMap<String, Move>,
    pub transition: Transition,
    pub child: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub chosen: Move,
    pub branches: Vec<Branch>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub state: State,
    pub remaining: u32,
    pub fixed: Option<Move>,
    pub label: Label,
    pub terminal: Option<String>,
    pub actions: Vec<Action>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Certificate {
    pub schema: u32,
    pub oracle: String,
    pub rules: String,
    pub focal: String,
    pub objective: Objective,
    pub horizon: u32,
    pub roots: [usize; 4],
    pub nodes: Vec<Node>,
}
pub struct Solution {
    pub labels: Labels,
    pub certificate: Option<Certificate>,
    pub explored: usize,
    pub cutoff: Option<String>,
}
pub struct Limits {
    pub nodes: usize,
    pub certificate: usize,
    pub wall: Duration,
}
struct Solver<'a> {
    focal: &'a str,
    objective: &'a Objective,
    limits: Limits,
    start: Instant,
    explored: usize,
    nodes: Vec<Node>,
    memo: HashMap<Vec<u8>, usize>,
    cutoff: Option<String>,
}
impl Solver<'_> {
    fn solve(&mut self, s: State, d: u32, fixed: Option<Move>) -> Option<usize> {
        let key = serde_json::to_vec(&(&s, d, fixed)).unwrap();
        if let Some(i) = self.memo.get(&key) {
            return Some(*i);
        }
        if self.explored >= self.limits.nodes
            || self.start.elapsed() >= self.limits.wall
            || self.nodes.len() >= self.limits.certificate
        {
            self.cutoff = Some("node/wall/certificate limit".into());
            return None;
        }
        self.explored += 1;
        let mut node = Node {
            state: s.clone(),
            remaining: d,
            fixed,
            label: Label::Unknown,
            terminal: None,
            actions: vec![],
        };
        if let Some((label, reason)) = s.terminal(self.focal, self.objective, d) {
            node.label = label;
            node.terminal = Some(reason);
        } else {
            let choices = fixed.map_or_else(|| Move::ALL.to_vec(), |m| vec![m]);
            let mut failures = vec![];
            let mut unresolved = false;
            for mv in choices {
                let mut action = Action {
                    chosen: mv,
                    branches: vec![],
                };
                let mut action_unknown = false;
                let mut defeated = false;
                for moves in rules::responses(&s, self.focal, mv) {
                    let transition = rules::advance(&s, &moves, self.focal, self.objective);
                    if let Some(child) = self.solve(transition.state.clone(), d - 1, None) {
                        let branch = Branch {
                            moves,
                            transition,
                            child,
                        };
                        if self.nodes[child].label == Label::Failure {
                            action.branches = vec![branch];
                            defeated = true;
                            break;
                        }
                        action.branches.push(branch);
                    } else {
                        action_unknown = true;
                    }
                }
                if defeated {
                    failures.push(action);
                } else if !action_unknown {
                    node.label = Label::Success;
                    node.actions = vec![action];
                    break;
                } else {
                    unresolved = true;
                }
            }
            if node.label != Label::Success {
                if unresolved {
                    return None;
                }
                node.label = Label::Failure;
                node.actions = failures;
            }
        }
        if self.nodes.len() >= self.limits.certificate {
            self.cutoff = Some("certificate limit".into());
            return None;
        }
        let id = self.nodes.len();
        self.nodes.push(node);
        self.memo.insert(key, id);
        Some(id)
    }
}
pub fn solve(r: &Request, o: &Objective, h: u32, limits: Limits) -> Result<Solution> {
    rules::validate(r)?;
    ensure!((1..=64).contains(&h), "horizon must be between 1 and 64");
    if let Objective::EatAndSurvive { food } = o {
        ensure!(
            !food.is_empty() && food.iter().all(|f| r.board.food.contains(f)),
            "objective requires designated initial food"
        );
    }
    let mut s = Solver {
        focal: &r.you.id,
        objective: o,
        limits,
        start: Instant::now(),
        explored: 0,
        nodes: vec![],
        memo: HashMap::new(),
        cutoff: None,
    };
    let mut roots = [0; 4];
    let mut labels = [Label::Unknown; 4];
    for mv in Move::ALL {
        if let Some(id) = s.solve(State::from_request(r), h, Some(mv)) {
            roots[mv.index()] = id;
            labels[mv.index()] = s.nodes[id].label;
        }
    }
    let cert = (!labels.contains(&Label::Unknown)).then(|| Certificate {
        schema: SCHEMA,
        oracle: ORACLE.into(),
        rules: RULES.into(),
        focal: r.you.id.clone(),
        objective: o.clone(),
        horizon: h,
        roots,
        nodes: s.nodes,
    });
    Ok(Solution {
        labels: Labels::from_array(labels),
        certificate: cert,
        explored: s.explored,
        cutoff: s.cutoff,
    })
}
/// Reconstruct labels by checking quantifier coverage and every transition. Stored
/// labels are only assertions to compare against reconstructed results.
pub fn verify(c: &Certificate, r: &Request) -> Result<Labels> {
    rules::validate(r)?;
    ensure!(
        c.schema == SCHEMA && c.oracle == ORACLE && c.rules == RULES,
        "incompatible certificate"
    );
    ensure!(
        (1..=64).contains(&c.horizon) && c.focal == r.you.id,
        "certificate focal/horizon mismatch"
    );
    if let Objective::EatAndSurvive { food } = &c.objective {
        ensure!(
            !food.is_empty() && food.iter().all(|p| r.board.food.contains(p)),
            "invalid objective"
        );
    }
    let mut done = HashMap::new();
    let mut visiting = HashSet::new();
    let mut labels = [Label::Unknown; 4];
    for mv in Move::ALL {
        let id = c.roots[mv.index()];
        let n = c
            .nodes
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("missing root"))?;
        ensure!(
            n.state == State::from_request(r) && n.remaining == c.horizon && n.fixed == Some(mv),
            "root state/action mismatch"
        );
        labels[mv.index()] = verify_node(c, id, &mut done, &mut visiting)?;
    }
    Ok(Labels::from_array(labels))
}
fn verify_node(
    c: &Certificate,
    id: usize,
    done: &mut HashMap<usize, Label>,
    visiting: &mut HashSet<usize>,
) -> Result<Label> {
    if let Some(l) = done.get(&id) {
        return Ok(*l);
    }
    ensure!(visiting.insert(id), "cyclic proof");
    let n = c
        .nodes
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("missing proof node"))?;
    let reconstructed = if let Some((l, reason)) =
        n.state.terminal(&c.focal, &c.objective, n.remaining)
    {
        ensure!(
            n.actions.is_empty() && n.terminal.as_ref() == Some(&reason),
            "invalid terminal proof"
        );
        l
    } else {
        ensure!(n.terminal.is_none(), "nonterminal node marked terminal");
        let choices = n.fixed.map_or_else(|| Move::ALL.to_vec(), |m| vec![m]);
        let mut by_move = BTreeMap::new();
        for a in &n.actions {
            ensure!(
                choices.contains(&a.chosen) && by_move.insert(a.chosen, ()).is_none(),
                "invalid/duplicate focal choice"
            );
            let all = rules::responses(&n.state, &c.focal, a.chosen);
            let mut seen = HashSet::new();
            let mut results = vec![];
            for b in &a.branches {
                let key = serde_json::to_vec(&b.moves)?;
                ensure!(
                    seen.insert(key) && all.contains(&b.moves),
                    "invalid/duplicate response"
                );
                let t = rules::advance(&n.state, &b.moves, &c.focal, &c.objective);
                ensure!(t == b.transition, "tampered transition");
                let child = c
                    .nodes
                    .get(b.child)
                    .ok_or_else(|| anyhow::anyhow!("missing child"))?;
                ensure!(
                    child.state == t.state
                        && child.remaining.checked_add(1) == Some(n.remaining)
                        && child.fixed.is_none(),
                    "child contract mismatch"
                );
                results.push(verify_node(c, b.child, done, visiting)?);
            }
            if n.label == Label::Success {
                ensure!(
                    a.branches.len() == all.len() && results.iter().all(|l| *l == Label::Success),
                    "missing universal success branches"
                );
            } else {
                ensure!(
                    a.branches.len() == 1 && results == [Label::Failure],
                    "failure lacks counterstrategy"
                );
            }
        }
        match n.label {
            Label::Success => {
                ensure!(n.actions.len() == 1, "success needs one strategy action");
                Label::Success
            }
            Label::Failure => {
                ensure!(
                    n.actions.len() == choices.len(),
                    "missing focal counterstrategy choices"
                );
                Label::Failure
            }
            Label::Unknown => bail!("unresolved proof"),
        }
    };
    ensure!(reconstructed == n.label, "incorrect stored label");
    visiting.remove(&id);
    done.insert(id, reconstructed);
    Ok(reconstructed)
}
/// Longest survival along the supplied counterstrategy (focal prolongs failure).
/// This is an example-policy diagnostic, not a claim about every opponent policy.
pub fn failure_delay(c: &Certificate, id: usize) -> u32 {
    let n = &c.nodes[id];
    if n.terminal.is_some() {
        return 0;
    }
    n.actions
        .iter()
        .flat_map(|a| a.branches.iter())
        .filter(|b| c.nodes[b.child].label == Label::Failure)
        .map(|b| 1 + failure_delay(c, b.child))
        .max()
        .unwrap_or(0)
}
/// One continuation selected from the full DAG. The DAG remains the proof.
pub fn example(c: &Certificate, m: Move) -> Vec<Transition> {
    let mut id = c.roots[m.index()];
    let mut out = vec![];
    while c.nodes[id].terminal.is_none() {
        let n = &c.nodes[id];
        let branch = n
            .actions
            .iter()
            .flat_map(|a| a.branches.iter())
            .max_by_key(|b| {
                if n.label == Label::Failure {
                    failure_delay(c, b.child)
                } else {
                    c.nodes[b.child].remaining
                }
            })
            .unwrap();
        out.push(branch.transition.clone());
        id = branch.child;
    }
    out
}
