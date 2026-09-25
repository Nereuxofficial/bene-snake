#!/usr/bin/env bash
set -euo pipefail

repo=${BENE_SNAKE_REPO:-"$HOME/containers/bene-snake"}
state_dir=${XDG_STATE_HOME:-"$HOME/.local/state"}/bene-snake
mkdir -p "$state_dir"
exec 9>"$state_dir/update.lock"
flock -n 9 || exit 0

cd "$repo"
if [[ -n $(git status --porcelain) ]]; then
    echo 'Working tree has local changes; skipping update' >&2
    exit 1
fi

git fetch origin main
git merge --ff-only FETCH_HEAD
revision=$(git rev-parse HEAD)
if [[ -f "$state_dir/deployed-revision" ]] && [[ $(cat "$state_dir/deployed-revision") == "$revision" ]]; then
    exit 0
fi

# Building changes only the image. The current container keeps serving games.
# A deferred deployment can reuse the image at the next timer tick.
image_name=$(docker compose config --images | head -1)
built_image=$(docker image inspect --format '{{.Id}}' "$image_name" 2>/dev/null || true)
if [[ ! -f "$state_dir/built-revision" ]] || [[ $(cat "$state_dir/built-revision") != "$revision" ]] ||
   [[ ! -f "$state_dir/built-image" ]] || [[ $(cat "$state_dir/built-image") != "$built_image" ]]; then
    GIT_REVISION="$revision" docker compose build bene-snake
    image_revision=$(docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' "$image_name")
    if [[ "$image_revision" != "$revision" ]]; then
        echo "Built image revision $image_revision does not match $revision" >&2
        exit 1
    fi
    printf '%s\n' "$revision" > "$state_dir/built-revision"
    docker image inspect --format '{{.Id}}' "$image_name" > "$state_dir/built-image"
fi

# 204 means no game is tracked and no game request arrived in the last minute.
# Recheck after the potentially long build, immediately before replacing the container.
port=$(sed -n 's/^PORT=//p' .env | tail -1)
port=${port:-8000}
status=$(curl --silent --output /dev/null --write-out '%{http_code}' --max-time 5 "http://127.0.0.1:$port/deploy-ready") || {
    echo 'Could not check game activity; keeping the current container' >&2
    exit 0
}
if [[ "$status" != 204 ]]; then
    echo "Deployment deferred: game activity check returned $status"
    exit 0
fi

previous_image=$(docker inspect --format '{{.Image}}' bene-snake)
docker tag "$previous_image" bene-snake:rollback
docker compose up -d --no-deps --no-build bene-snake

for _ in {1..15}; do
    if response=$(curl --silent --fail --max-time 2 "http://127.0.0.1:$port/"); then
        reported_revision=$(python3 -c 'import json, sys; print(json.load(sys.stdin).get("rev", ""))' <<< "$response" 2>/dev/null || true)
        if [[ "$reported_revision" == "$revision" ]]; then
            printf '%s\n' "$revision" > "$state_dir/deployed-revision"
            echo "Deployed $revision"
            exit 0
        fi
    fi
    sleep 2
done

echo 'New container failed its revision check; restoring the previous image' >&2
docker tag bene-snake:rollback "$image_name"
docker compose up -d --no-deps --no-build --force-recreate bene-snake
exit 1
