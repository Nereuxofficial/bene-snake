# bene-snake

# Running the project
After [installing Rust](https://rustup.rs), run the following command in the project directory:
```
cargo run --release
```

## Automatic deployment on nixfix

The `bene-snake-update.timer` user timer checks `origin/main` every five minutes.
When there is a new commit, `deploy/update.sh` builds its Docker image while the
current container stays online. It replaces the container only when
`/deploy-ready` returns HTTP 204: every observed game has received `/end`, and
no `/start`, `/move`, or `/end` request has arrived for at least 60 seconds.
If the activity check fails or reports a game, the image remains ready for the
next timer run. A failed HTTP check after replacement restores the old image.

On nixfix, inspect the timer with `systemctl --user status bene-snake-update.timer`
and its output with `journalctl --user -u bene-snake-update.service`.
