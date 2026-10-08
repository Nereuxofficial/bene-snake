use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use gym::{artifact::*, generate, report, runner};
use std::{io::Read, net::SocketAddr, path::PathBuf};
#[derive(Parser)]
#[command(
    name = "snake-gym",
    version,
    about = "Certified paired production decision benchmark (synthetic, no future food, cold trees)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Freeze a fresh certified corpus, then compare two production HTTP candidates
    Compare {
        #[arg(long)]
        a: String,
        #[arg(long)]
        b: String,
        #[arg(long)]
        corpus: Option<PathBuf>,
        #[arg(long)]
        suite: Option<String>,
        #[arg(long)]
        cases: Option<usize>,
        #[arg(long)]
        seed: Option<u64>,
        #[arg(long)]
        repeats: Option<usize>,
        #[arg(long, default_value_t = 500)]
        timeout_ms: u64,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Generate/certify positions without consulting candidates
    Generate {
        #[arg(long, default_value = "balanced-v1")]
        suite: String,
        #[arg(long, default_value_t = 240)]
        cases: usize,
        #[arg(long)]
        seed: Option<u64>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Resume the saved schedule; verify all immutable identities first
    Resume { run: PathBuf },
    /// Rebuild summary and offline report from saved records
    Report { run: PathBuf },
    /// Serve a read-only report on loopback
    Serve {
        run: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8050")]
        bind: SocketAddr,
    },
    /// Inspect a case's wire request, assumptions and certified labels
    Inspect {
        run: PathBuf,
        #[arg(long)]
        case: String,
    },
    /// Reconstruct every label from full certificates
    VerifyCorpus { corpus: PathBuf },
}
fn seed(value: Option<u64>) -> Result<u64> {
    if let Some(v) = value {
        return Ok(v);
    }
    let mut bytes = [0; 8];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}
fn finish(out: &std::path::Path) -> Result<()> {
    let path = report::export(out)?;
    let s = runner::summary(out)?;
    println!(
        "{}: A={:?}, B={:?}, delta={:?}; {}",
        s.status, s.accuracy_a, s.accuracy_b, s.delta, s.verdict
    );
    println!(
        "Cold production decisions; synthetic distribution; no future food; adversarial coalition."
    );
    println!("Report: {}", path.canonicalize()?.display());
    println!(
        "Rebuild: {} report {}",
        out.join("binaries/snake-gym").display(),
        out.display()
    );
    println!(
        "Resume: {} resume {}",
        out.join("binaries/snake-gym").display(),
        out.display()
    );
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Generate {
            suite,
            cases,
            seed: value,
            out,
        } => {
            let seed = seed(value)?;
            println!("Seed: {seed}");
            let g = generate::generate(&out, generate::suite(&suite)?, cases, seed)?;
            println!(
                "Corpus: {}; complete={}; {} ms",
                out.canonicalize()?.display(),
                g.complete,
                g.elapsed_ms
            );
            ensure!(g.complete, "quota shortage; inspect generation.json");
        }
        Command::Compare {
            a,
            b,
            corpus,
            suite,
            cases,
            seed: value,
            repeats,
            timeout_ms,
            out,
        } => {
            ensure!(
                corpus.is_none() || suite.is_none() && cases.is_none() && value.is_none(),
                "--corpus conflicts with --suite/--cases/--seed"
            );
            let suite_name = suite.as_deref().unwrap_or("balanced-v1");
            let screening = suite_name == "screening-v1";
            let hard = suite_name == "hard-v1" || suite_name == "hard-confirmation-v1";
            let repeats = repeats.unwrap_or(if screening { 1 } else { 3 });
            let out = out.unwrap_or_else(|| PathBuf::from(format!("gym-runs/{}", timestamp())));
            ensure!(
                !out.join("manifest.json").exists(),
                "run already exists; use resume"
            );
            std::fs::create_dir_all(&out)?;
            let out = out.canonicalize()?;
            if let Some(corpus) = corpus {
                runner::freeze_corpus(&corpus, &out)?;
            } else {
                let seed = seed(value)?;
                println!("Seed: {seed}");
                let g = generate::generate(
                    &out,
                    generate::suite(suite_name)?,
                    cases.unwrap_or(if screening || hard { 60 } else { 240 }),
                    seed,
                )?;
                ensure!(g.complete, "quota shortage; candidates were not consulted");
            }
            println!(
                "Request allowance: {:.1}s plus startup, builds and cleanup",
                generate::load(&out)?.1.len() as f64 * repeats as f64 * 2.0 * timeout_ms as f64
                    / 1000.0
            );
            runner::initialize(&std::env::current_dir()?, &out, &a, &b, repeats, timeout_ms)?;
            let result = runner::execute(&out, false).await;
            let exported = finish(&out);
            result?;
            exported?;
        }
        Command::Resume { run } => {
            let out = run.canonicalize()?;
            let result = runner::execute(&out, true).await;
            let exported = finish(&out);
            result?;
            exported?;
        }
        Command::Report { run } => finish(&run)?,
        Command::Serve { run, bind } => report::serve(&run, bind).await?,
        Command::Inspect { run, case } => {
            let (_, cases) = generate::load(&run)?;
            let c = cases
                .iter()
                .find(|c| c.id == case)
                .ok_or_else(|| anyhow::anyhow!("unknown case"))?;
            println!("{}", serde_json::to_string_pretty(c)?);
        }
        Command::VerifyCorpus { corpus } => {
            let (g, cases) = generate::load(&corpus)?;
            println!(
                "Verified {} certificates; complete={}; seed={}",
                cases.len(),
                g.complete,
                g.seed
            );
            ensure!(g.complete, "incomplete quotas");
        }
    }
    Ok(())
}
