fn main() -> anyhow::Result<()> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or("/tmp/gym-generation-check".into());
    let n = std::env::args().nth(2).unwrap_or("6".into()).parse()?;
    let seed = std::env::args().nth(3).unwrap_or("1234".into()).parse()?;
    let g = gym::generate::generate(
        std::path::Path::new(&out),
        gym::generate::suite("balanced-v1")?,
        n,
        seed,
    )?;
    println!("complete: {}, elapsed {} ms", g.complete, g.elapsed_ms);
    Ok(())
}
