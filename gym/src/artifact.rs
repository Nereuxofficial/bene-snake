//! Content identities and atomic/durable artifact I/O.
use anyhow::{Context, Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
pub fn hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
pub fn file_hash(path: &Path) -> Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut b = [0; 65536];
    loop {
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
pub fn value_hash(v: &impl Serialize) -> Result<String> {
    Ok(hash(&serde_json::to_vec(v)?))
}
pub fn atomic(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().context("no parent")?;
    fs::create_dir_all(parent)?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut f = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::rename(tmp, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn write_json(path: &Path, v: &impl Serialize) -> Result<()> {
    atomic(path, &serde_json::to_vec_pretty(v)?)
}
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path)?).with_context(|| format!("reading {}", path.display()))
}
pub fn read_jsonl<T: DeserializeOwned>(path: &Path, recover: bool) -> Result<Vec<T>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let bytes = fs::read(path)?;
    let mut rows = vec![];
    let mut offset = 0;
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        let complete = line.ends_with(b"\n");
        if !complete && recover {
            let f = OpenOptions::new().write(true).open(path)?;
            f.set_len(offset as u64)?;
            f.sync_all()?;
            break;
        }
        ensure!(complete, "truncated corpus JSONL");
        ensure!(line.len() > 1, "empty JSONL record");
        rows.push(
            serde_json::from_slice(line)
                .with_context(|| format!("corrupt JSONL at byte {offset}"))?,
        );
        offset += line.len();
    }
    Ok(rows)
}
pub fn append(path: &Path, v: &impl Serialize) -> Result<()> {
    let mut f = OpenOptions::new().append(true).create(true).open(path)?;
    serde_json::to_writer(&mut f, v)?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    Ok(())
}
pub fn timestamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}
/// SplitMix64 v1, explicit integer algorithm with fixed constants. This stream is
/// independent of rand crate versions and reproducible across supported hosts.
pub struct Rng(pub u64);
impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    pub fn range(&mut self, n: usize) -> usize {
        ((self.next_u64() as u128 * n as u128) >> 64) as usize
    }
}
pub fn stream(seed: u64, family: &str, index: usize, attempt: usize) -> u64 {
    let bytes =
        Sha256::digest(serde_json::to_vec(&(GENERATOR, seed, family, index, attempt)).unwrap());
    u64::from_le_bytes(bytes[..8].try_into().unwrap())
}
use crate::schema::GENERATOR;
/// Frozen harness identities include implementation source, not just version tags.
pub fn rules_hash() -> String {
    hash(include_bytes!("rules.rs"))
}
pub fn oracle_hash() -> String {
    hash(include_bytes!("oracle/mod.rs"))
}
pub fn generator_hash() -> String {
    hash(
        &[
            include_bytes!("generate.rs").as_slice(),
            include_bytes!("hard.rs").as_slice(),
        ]
        .concat(),
    )
}
