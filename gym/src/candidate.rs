//! Resolve immutable refs without changing the checkout; freeze all executables.
use crate::{artifact::*, schema::Candidate};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn output(mut c: Command) -> Result<String> {
    let o = c.output()?;
    ensure!(
        o.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    Ok(String::from_utf8(o.stdout)?.trim().to_string())
}
fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let mut c = Command::new("git");
    c.current_dir(repo).args(args);
    output(c)
}
pub fn machine() -> Result<String> {
    let mut c = Command::new("uname");
    c.arg("-a");
    let os = output(c)?;
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("model name"))
        .unwrap_or("unknown CPU")
        .to_string();
    Ok(format!(
        "{os}; {cpu}; logical_cpus={}",
        std::thread::available_parallelism()?.get()
    ))
}
pub fn prepare(repo: &Path, run: &Path, descriptor: &str, index: usize) -> Result<Candidate> {
    fs::create_dir_all(run.join("binaries"))?;
    fs::create_dir_all(run.join("logs"))?;
    let frozen = run.join("binaries").join(format!("candidate-{index}"));
    ensure!(!frozen.exists(), "candidate already frozen");
    let mut candidate = Candidate {
        descriptor: descriptor.into(),
        sha: None,
        source_hash: None,
        lock_hash: None,
        binary_hash: String::new(),
        binary: format!("binaries/candidate-{index}"),
        toolchain: "unobservable for supplied binary".into(),
        flags: vec![],
        build_log: None,
        submodules: String::new(),
    };
    let source = if let Some(path) = descriptor.strip_prefix("bin:") {
        let path = PathBuf::from(path);
        ensure!(
            path.is_absolute() && path.is_file(),
            "binary must be an absolute file path"
        );
        candidate.toolchain = "unobservable for supplied binary".into();
        path
    } else if let Some(reference) = descriptor.strip_prefix("ref:") {
        ensure!(!reference.is_empty(), "empty ref");
        let sha = git(
            repo,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{reference}^{{commit}}"),
            ],
        )?;
        candidate.sha = Some(sha.clone());
        let mut archive = Command::new("git");
        archive
            .current_dir(repo)
            .args(["archive", "--format=tar", &sha]);
        let archive = archive.output()?;
        ensure!(archive.status.success(), "source archive failed");
        let source_hash = hash(&archive.stdout);
        candidate.source_hash = Some(source_hash.clone());
        // Include exact submodule identities; unsupported unavailable submodules fail explicitly.
        let tree = git(repo, &["ls-tree", "-r", &sha])?;
        let submodules = tree
            .lines()
            .filter(|l| l.starts_with("160000"))
            .collect::<Vec<_>>()
            .join("\n");
        candidate.submodules = submodules.clone();
        ensure!(
            submodules.is_empty(),
            "ref contains submodules; archive mode cannot build them: {submodules}"
        );
        let flags = vec![
            "--release".into(),
            "--locked".into(),
            "-p".into(),
            "bene-snake".into(),
        ];
        candidate.flags = flags.clone();
        let src = run.join("builds/inputs").join(&source_hash);
        fs::create_dir_all(&src)?;
        let mut tar = Command::new("tar")
            .args(["-xf", "-", "-C"])
            .arg(&src)
            .stdin(Stdio::piped())
            .spawn()?;
        tar.stdin.take().unwrap().write_all(&archive.stdout)?;
        ensure!(tar.wait()?.success(), "extract failed");
        let rustc = {
            let mut c = Command::new("rustc");
            c.current_dir(&src).arg("-Vv");
            output(c)?
        };
        candidate.toolchain = rustc.clone();
        let key = value_hash(&(&source_hash, &rustc, &flags))?;
        let target = run.join("builds").join(key).join("target");
        candidate.lock_hash = Some(file_hash(&src.join("Cargo.lock"))?);
        let log = run.join("logs").join(format!("build-{index}.log"));
        let file = fs::File::create(&log)?;
        let status = Command::new("cargo")
            .current_dir(&src)
            .args(["build", "--release", "--locked", "-p", "bene-snake"])
            .env("CARGO_TARGET_DIR", &target)
            .env("RUSTC_WRAPPER", "")
            .env("GIT_REVISION", &sha)
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .stdout(file.try_clone()?)
            .stderr(file)
            .status()?;
        candidate.build_log = Some(format!("logs/build-{index}.log"));
        ensure!(
            status.success(),
            "ref build failed; inspect {}",
            log.display()
        );
        target.join("release/bene-snake")
    } else {
        anyhow::bail!("candidate must be ref:<git-ref> or bin:<absolute-path>")
    };
    fs::copy(&source, &frozen).with_context(|| format!("freezing {}", source.display()))?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&frozen, fs::Permissions::from_mode(0o500))?;
    candidate.binary_hash = file_hash(&frozen)?;
    Ok(candidate)
}
pub fn check(run: &Path, c: &Candidate) -> Result<()> {
    ensure!(
        file_hash(&run.join(&c.binary))? == c.binary_hash,
        "frozen binary hash mismatch: {}",
        c.descriptor
    );
    Ok(())
}
