#!/usr/bin/env bash
set -euo pipefail
umask 077
image="${1:?image required}"
version="$(docker run --rm --network none --entrypoint /usr/local/bin/mail "$image" --version)"
if [ -n "${2:-}" ]; then test "${version##* }" = "$2"; fi
printf '%s\n' "$version"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/ha-mail-smoke.XXXXXX")
container="ha-mail-smoke-${RANDOM}-${RANDOM}"
volume="$container-data"
restore_volume="$container-restored"
cleanup() {
    status=$?
    if [ "$status" -ne 0 ]; then docker logs "$container" >&2 || true; fi
    docker rm -f "$container" >/dev/null 2>&1 || true
    docker volume rm "$volume" "$restore_volume" >/dev/null 2>&1 || true
    rm -rf "$scratch"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Use the actual entrypoint and Supervisor options path. This fictional,
# unbound mailbox cannot fetch provider data; networking is disabled as well.
printf '%s\n' '{"stable_api_key":"fictional-provider-key","environment":"sandbox"}' >"$scratch/options.json"
docker volume create "$volume" >/dev/null
docker create --name "$container" --network none \
    --mount "type=volume,source=$volume,target=/data" \
    -e MAIL_ENABLE_INGRESS=false "$image" >/dev/null
docker cp "$scratch/options.json" "$container:/data/options.json"

wait_for_api() {
    for _ in {1..30}; do
        if docker exec "$container" wget -qO- http://127.0.0.1:8080/health >/dev/null; then
            return
        fi
        sleep 1
    done
    echo 'Mail did not start through its production entrypoint' >&2
    return 1
}

check_access() {
    docker cp "$container:/data/credentials.json" "$scratch/current-credentials.json"
    local owner integration
    owner=$(jq -er '.owner' "$scratch/current-credentials.json")
    integration=$(jq -er '.integration' "$scratch/current-credentials.json")
    test "$owner" != "$integration"
    for credential in "$owner" "$integration"; do
        docker exec "$container" wget -qO- --header "Authorization: Bearer $credential" \
            http://127.0.0.1:8080/v1/system >"$scratch/system.json"
        jq -e '.environment == "sandbox"' "$scratch/system.json" >/dev/null
    done
    docker exec "$container" sh -c 'test "$(stat -c %a /data/mail.sqlite3)" = 600 && test "$(stat -c %a /data/credentials.json)" = 600'
    if docker exec "$container" wget -qO- http://127.0.0.1:8080/v1/items; then
        echo 'Unauthenticated mail read was accepted' >&2
        return 1
    fi
}

stop_container() {
    docker stop --time 10 "$container" >/dev/null
    test "$(docker inspect "$container" --format '{{.State.ExitCode}}')" = 0
}

docker start "$container" >/dev/null
wait_for_api
check_access
cp "$scratch/current-credentials.json" "$scratch/original-credentials.json"
stop_container
mkdir "$scratch/private-data"
docker cp "$container:/data/." "$scratch/private-data"
docker rm "$container" >/dev/null
docker volume create "$restore_volume" >/dev/null
docker create --name "$container" --network none \
    --mount "type=volume,source=$restore_volume,target=/data" \
    -e MAIL_ENABLE_INGRESS=false "$image" >/dev/null
docker cp "$scratch/private-data/." "$container:/data"
docker start "$container" >/dev/null
wait_for_api
check_access
cmp -s "$scratch/original-credentials.json" "$scratch/current-credentials.json"
stop_container
echo 'Container smoke passed: production startup, authenticated access, private modes, and credential-preserving cold restore.'
