use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};

mod agents;
mod runner;
mod stats;

use lib::Agent;
use agents::{HeuristicAgent, MctsAgent, MinimaxAgent, RandomAgent};
use runner::{run_game, GameConfig};
use stats::{HeadToHeadStats, TournamentStats};

#[derive(Parser)]
#[command(name = "snake-gym")]
#[command(about = "Benchmarking gym for pitting bene-snake against other snake implementations")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a tournament between multiple agents
    Tournament {
        /// Number of games to run
        #[arg(short, long, default_value = "100")]
        games: usize,

        /// Agents to include in the tournament
        #[arg(short, long, value_delimiter = ',', default_value = "mcts,random,heuristic")]
        agents: Vec<AgentType>,

        /// MCTS think time in milliseconds
        #[arg(long, default_value = "50")]
        mcts_time: u64,

        /// Minimax search depth
        #[arg(long, default_value = "3")]
        minimax_depth: u32,

        /// Maximum turns per game
        #[arg(long, default_value = "500")]
        max_turns: u32,

        /// Run games in parallel
        #[arg(short, long)]
        parallel: bool,

        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },

    /// Run a head-to-head duel between two agents
    Duel {
        /// First agent
        #[arg(short = '1', long, default_value = "mcts")]
        agent1: AgentType,

        /// Second agent
        #[arg(short = '2', long, default_value = "random")]
        agent2: AgentType,

        /// Number of games to run
        #[arg(short, long, default_value = "100")]
        games: usize,

        /// MCTS think time in milliseconds
        #[arg(long, default_value = "50")]
        mcts_time: u64,

        /// Minimax search depth
        #[arg(long, default_value = "3")]
        minimax_depth: u32,

        /// Maximum turns per game
        #[arg(long, default_value = "500")]
        max_turns: u32,

        /// Run games in parallel
        #[arg(short, long)]
        parallel: bool,

        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },

    /// Run a quick benchmark to test performance
    Benchmark {
        /// Number of games to run
        #[arg(short, long, default_value = "10")]
        games: usize,

        /// MCTS think times to test (in ms)
        #[arg(long, value_delimiter = ',', default_value = "10,25,50,100")]
        mcts_times: Vec<u64>,

        /// Run games in parallel
        #[arg(short, long)]
        parallel: bool,
    },
}

#[derive(Clone, Copy, ValueEnum, Debug, PartialEq)]
enum AgentType {
    Mcts,
    Random,
    Heuristic,
    Minimax,
}

impl AgentType {
    fn create_agent(&self, mcts_time_ms: u64, minimax_depth: u32) -> Box<dyn Agent> {
        match self {
            AgentType::Mcts => Box::new(MctsAgent::new(Duration::from_millis(mcts_time_ms))),
            AgentType::Random => Box::new(RandomAgent::new()),
            AgentType::Heuristic => Box::new(HeuristicAgent::new()),
            AgentType::Minimax => Box::new(MinimaxAgent::new(minimax_depth)),
        }
    }
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Tournament {
            games,
            agents,
            mcts_time,
            minimax_depth,
            max_turns,
            parallel,
            json,
        } => {
            run_tournament_cmd(games, &agents, mcts_time, minimax_depth, max_turns, parallel, json);
        }
        Commands::Duel {
            agent1,
            agent2,
            games,
            mcts_time,
            minimax_depth,
            max_turns,
            parallel,
            json,
        } => {
            run_duel_cmd(agent1, agent2, games, mcts_time, minimax_depth, max_turns, parallel, json);
        }
        Commands::Benchmark {
            games,
            mcts_times,
            parallel,
        } => {
            run_benchmark_cmd(games, &mcts_times, parallel);
        }
    }
}

fn run_tournament_cmd(
    num_games: usize,
    agent_types: &[AgentType],
    mcts_time: u64,
    minimax_depth: u32,
    max_turns: u32,
    parallel: bool,
    json_output: bool,
) {
    if !json_output {
        println!(
            "\n{}",
            "=== Snake Gym Tournament ===".green().bold()
        );
        println!("Games: {} | Max turns: {}", num_games, max_turns);
        println!("Parallel: {} | MCTS time: {}ms", parallel, mcts_time);
        println!();
    }

    // Create agents
    let agents: Vec<Box<dyn Agent>> = agent_types
        .iter()
        .map(|t| t.create_agent(mcts_time, minimax_depth))
        .collect();

    let agent_refs: Vec<&dyn Agent> = agents.iter().map(|a| a.as_ref()).collect();
    let agent_names: Vec<String> = agents.iter().map(|a| a.name().to_string()).collect();

    let config = GameConfig {
        num_snakes: agents.len().min(4),
        max_turns,
        ..GameConfig::default()
    };

    // Progress bar
    let pb = if !json_output {
        let pb = ProgressBar::new(num_games as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        Some(pb)
    } else {
        None
    };

    // Run games
    let results: Vec<_> = if parallel {
        use rayon::prelude::*;
        (0..num_games)
            .into_par_iter()
            .map(|_| {
                let result = run_game(&agent_refs, &config);
                if let Some(ref pb) = pb {
                    pb.inc(1);
                }
                result
            })
            .collect()
    } else {
        (0..num_games)
            .map(|_| {
                let result = run_game(&agent_refs, &config);
                if let Some(ref pb) = pb {
                    pb.inc(1);
                }
                result
            })
            .collect()
    };

    if let Some(pb) = pb {
        pb.finish_with_message("Done!");
    }

    // Compute and display stats
    let stats = TournamentStats::from_results(&results, &agent_names);

    if json_output {
        println!("{}", stats.to_json());
    } else {
        stats.print_summary();
    }
}

fn run_duel_cmd(
    agent1_type: AgentType,
    agent2_type: AgentType,
    num_games: usize,
    mcts_time: u64,
    minimax_depth: u32,
    max_turns: u32,
    parallel: bool,
    json_output: bool,
) {
    if !json_output {
        println!("\n{}", "=== Snake Gym Duel ===".green().bold());
        println!(
            "{:?} vs {:?}",
            agent1_type, agent2_type
        );
        println!("Games: {} | Max turns: {}", num_games, max_turns);
        println!();
    }

    // Create agents
    let agent1 = agent1_type.create_agent(mcts_time, minimax_depth);
    let agent2 = agent2_type.create_agent(mcts_time, minimax_depth);
    let agents: Vec<&dyn Agent> = vec![agent1.as_ref(), agent2.as_ref()];

    let config = GameConfig::duel().with_max_turns(max_turns);

    // Progress bar
    let pb = if !json_output {
        let pb = ProgressBar::new(num_games as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        Some(pb)
    } else {
        None
    };

    // Run games
    let results: Vec<_> = if parallel {
        use rayon::prelude::*;
        (0..num_games)
            .into_par_iter()
            .map(|_| {
                let result = run_game(&agents, &config);
                if let Some(ref pb) = pb {
                    pb.inc(1);
                }
                result
            })
            .collect()
    } else {
        (0..num_games)
            .map(|_| {
                let result = run_game(&agents, &config);
                if let Some(ref pb) = pb {
                    pb.inc(1);
                }
                result
            })
            .collect()
    };

    if let Some(pb) = pb {
        pb.finish_with_message("Done!");
    }

    // Compute and display stats
    let h2h = HeadToHeadStats::from_results(&results, agent1.name(), agent2.name());

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "agent1": {
                    "name": h2h.agent1_name,
                    "wins": h2h.agent1_wins,
                },
                "agent2": {
                    "name": h2h.agent2_name,
                    "wins": h2h.agent2_wins,
                },
                "draws": h2h.draws,
                "total_games": num_games,
            }))
            .unwrap()
        );
    } else {
        h2h.print_summary();
    }
}

fn run_benchmark_cmd(games_per_config: usize, mcts_times: &[u64], parallel: bool) {
    println!("\n{}", "=== Snake Gym Benchmark ===".green().bold());
    println!(
        "Testing MCTS at different think times against Random baseline"
    );
    println!("Games per config: {}", games_per_config);
    println!();

    let random_agent = RandomAgent::new();

    for &time_ms in mcts_times {
        let mcts_agent = MctsAgent::with_name(format!("MCTS-{}ms", time_ms), Duration::from_millis(time_ms));

        let agents: Vec<&dyn Agent> = vec![&mcts_agent, &random_agent];
        let config = GameConfig::duel();

        let pb = ProgressBar::new(games_per_config as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(&format!(
                    "{{spinner:.green}} MCTS {}ms [{{bar:30.cyan/blue}}] {{pos}}/{{len}}",
                    time_ms
                ))
                .unwrap()
                .progress_chars("#>-"),
        );

        let results: Vec<_> = if parallel {
            use rayon::prelude::*;
            (0..games_per_config)
                .into_par_iter()
                .map(|_| {
                    let result = run_game(&agents, &config);
                    pb.inc(1);
                    result
                })
                .collect()
        } else {
            (0..games_per_config)
                .map(|_| {
                    let result = run_game(&agents, &config);
                    pb.inc(1);
                    result
                })
                .collect()
        };

        pb.finish();

        let h2h = HeadToHeadStats::from_results(&results, mcts_agent.name(), random_agent.name());

        let win_rate = h2h.agent1_wins as f64 / (h2h.agent1_wins + h2h.agent2_wins + h2h.draws) as f64 * 100.0;

        println!(
            "  MCTS {}ms: {:.1}% win rate ({} wins / {} losses / {} draws)",
            time_ms,
            win_rate,
            h2h.agent1_wins.to_string().green(),
            h2h.agent2_wins.to_string().red(),
            h2h.draws.to_string().yellow()
        );
    }

    println!();
}

// Extension trait for GameConfig
impl GameConfig {
    fn with_max_turns(mut self, max_turns: u32) -> Self {
        self.max_turns = max_turns;
        self
    }
}
