#!/usr/bin/env bash
# Runs check-server-compat.mjs against throwaway repos with tagged rpc.rs variants
# and compares the exit codes and messages with scripts/check-server-compat.golden.txt.
# BLESS=1 rewrites the golden; review its diff like code.
set -euo pipefail

cd "$(dirname "$0")/.."
CHECK="$PWD/scripts/check-server-compat.mjs"
GOLDEN="$PWD/scripts/check-server-compat.golden.txt"
OUT_DIR="$PWD/target/server-compat"
RESULTS="$OUT_DIR/results.txt"
mkdir -p "$OUT_DIR"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/server-compat.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

# Hermetic git: no user signing config, so scratch commits and tags never prompt.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t

# The default table; each case edits the copy it runs in.
TABLE='    GetThing: get_thing(id: String) -> Thing;
    ListThings: list_things(
        project: Option<String>,
        limit: u32,
    ) -> Vec<Thing>;
    SetFlag: set_flag(on: bool) -> ();
    GetStatus: get_status() -> Status;
    Ping: ping(nonce: u64) -> u64;
    ServerOnly: server_only() -> ();'

table() {
  cat > crates/hyprlayer-server/src/rpc.rs <<EOF
use crate::client::RpcClient;

hyprlayer_proto::commands! {
    client = RpcClient;

$1
}
EOF
}
server_version() { printf '[package]\nname = "hyprlayer-server"\nversion = "%s"\n' "$1" > crates/hyprlayer-server/Cargo.toml; }
min_version() { printf 'pub const MIN_SERVER_VERSION: &str = "%s";\n' "$1" > crates/hyprlayer-client/src/server_version.rs; }
commit() { git add -A && git commit -qm "$1"; }
proto() { mkdir -p crates/hyprlayer-proto/src && printf 'pub const NEW_FEATURE: &str = "%s";\n' "$1" > crates/hyprlayer-proto/src/lib.rs; }
gated() { printf '// A note.\n\n// Another.\nfn f(c: RpcClient) {\n    // server-compat: gated on %s\n    c.new_thing();\n}\n' "$1" > src-tauri/src/commands/gated.rs; }
# Fails when the substitution matches nothing, so a stale pattern can't turn a case into a no-op.
replace() {
  local before
  before="$(cat "$1")"
  perl -0pi -e "$2" "$1"
  [[ "$(cat "$1")" != "$before" ]] || { echo "replace matched nothing in $1: $2" >&2; return 1; }
}

mkdir -p "$WORK/base" && cd "$WORK/base"
git init -q
mkdir -p crates/hyprlayer-server/src crates/hyprlayer-client/src crates/hyprlayer-desktop/src crates/hyprlayer-ui/src src-tauri/src/commands
table "$TABLE"
server_version 0.9.0
min_version 0.9.0
cat > src-tauri/src/commands/things.rs <<'EOF'
pub async fn get_thing(state: State<'_, AppState>, id: String) -> CmdResult<Thing> {
    Ok(state.client()?.get_thing(id).await?)
}

pub async fn list_things(state: State<'_, AppState>, project: Option<String>) -> CmdResult<Vec<Thing>> {
    Ok(state
        .client()?
        .list_things(project, 50)
        .await?)
}
EOF
cat > src-tauri/src/commands/helpers.rs <<'EOF'
async fn flag(client: Arc<RpcClient>) -> Result<(), WireError> {
    client.set_flag(true).await
}
EOF
cat > src-tauri/src/lib.rs <<'EOF'
// Later this could call client.new_thing() once the server has it.
async fn connected(client: Arc<RpcClient>) {
    let _ = client.get_status().await;
    let _ = client.call(Request::Ping { nonce: 1 }).await;
}
EOF
commit "server 0.9.0"
git tag server-v0.9.0
server_version 0.10.0
commit "start 0.10.0"
cd "$WORK"

: > "$RESULTS"
# Runs the setup shell code in a fresh copy of base, then the check.
case_() {
  local name="$1" setup="$2"
  rm -rf "$WORK/$name" && cp -R "$WORK/base" "$WORK/$name"
  (cd "$WORK/$name" && set -e && eval "$setup") || { echo "✗ setup failed for $name"; exit 1; }
  local code=0 out
  out="$(node "$CHECK" --root "$WORK/$name" 2>&1)" || code=$?
  printf '## %s\nexit: %s\n%s\n\n' "$name" "$code" "$out" >> "$RESULTS"
}

case_ baseline ''
case_ new_command_app_calls "table \"\$TABLE
    NewThing: new_thing() -> Thing;\"
  echo 'fn f(c: RpcClient) { c.new_thing(); }' > src-tauri/src/commands/new.rs"
case_ new_command_client_crate_calls "table \"\$TABLE
    NewThing: new_thing() -> Thing;\"
  echo 'fn f(c: RpcClient) { c.new_thing(); }' > crates/hyprlayer-client/src/new.rs"
case_ new_command_server_only_with_comment_mention "table \"\$TABLE
    NewThing: new_thing() -> Thing;\""
case_ arg_type_changed "replace crates/hyprlayer-server/src/rpc.rs 's/get_thing\(id: String\)/get_thing(id: u64)/'"
case_ arg_renamed "replace crates/hyprlayer-server/src/rpc.rs 's/get_thing\(id: String\)/get_thing(thing_id: String)/'"
case_ return_type_changed "replace crates/hyprlayer-server/src/rpc.rs 's/-> Thing;/-> Option<Thing>;/'"
case_ variant_renamed "replace crates/hyprlayer-server/src/rpc.rs 's/GetThing: get_thing/FetchThing: get_thing/'"
case_ method_renamed "replace crates/hyprlayer-server/src/rpc.rs 's/get_thing\(/fetch_thing(/'
  replace src-tauri/src/commands/things.rs 's/\.get_thing\(/.fetch_thing(/'"
case_ multiline_entry_arg_changed "replace crates/hyprlayer-server/src/rpc.rs 's/limit: u32,/limit: u64,/'"
case_ multiline_entry_reflowed_only "replace crates/hyprlayer-server/src/rpc.rs 's/ListThings: list_things\(\s*project: Option<String>,\s*limit: u32,\s*\) -> Vec<Thing>;/ListThings: list_things(project: Option<String>, limit: u32) -> Vec< Thing >;/'"
case_ trailing_comment_on_entry "replace crates/hyprlayer-server/src/rpc.rs 's/(SetFlag: set_flag\(on: bool\) -> \(\);)/\$1 \/\/ toggles the flag/'"
case_ helper_wrapper_call_changed "replace crates/hyprlayer-server/src/rpc.rs 's/set_flag\(on: bool\)/set_flag(on: bool, why: String)/'"
case_ non_commands_file_call_changed "replace crates/hyprlayer-server/src/rpc.rs 's/-> Status;/-> Option<Status>;/'"
case_ request_dispatch_changed "replace crates/hyprlayer-server/src/rpc.rs 's/ping\(nonce: u64\)/ping(nonce: u32)/'"
case_ server_only_command_changed "replace crates/hyprlayer-server/src/rpc.rs 's/server_only\(\)/server_only(force: bool)/'"
case_ command_removed "replace crates/hyprlayer-server/src/rpc.rs 's/ +ServerOnly: server_only\(\) -> \(\);\n//'"
case_ min_tag_missing 'min_version 0.8.0'
case_ min_equals_untagged_server "min_version 0.10.0
  table \"\$TABLE
    NewThing: new_thing() -> Thing;\"
  echo 'fn f(c: RpcClient) { c.new_thing(); }' > src-tauri/src/commands/new.rs"
case_ min_above_server 'min_version 0.11.0'
case_ released_server_min_not_raised "table \"\$TABLE
    NewThing: new_thing() -> Thing;\"
  echo 'fn f(c: RpcClient) { c.new_thing(); }' > src-tauri/src/commands/new.rs
  commit 'server 0.10.0'
  git tag server-v0.10.0"
case_ table_drift_without_bump "git tag server-v0.10.0
  table \"\$TABLE
    NewThing: new_thing() -> Thing;\""
case_ table_drift_without_bump_app_calls_old_min "git tag server-v0.10.0
  table \"\$TABLE
    NewThing: new_thing() -> Thing;\"
  echo 'fn f(c: RpcClient) { c.new_thing(); }' > src-tauri/src/commands/new.rs"
case_ gated_call_on_a_feature_min_lacks "table \"\$TABLE
    NewThing: new_thing() -> Thing;\"
  proto new_feature
  gated new_feature"
case_ gated_call_on_a_feature_min_lists "proto new_feature
  commit 'list new_feature'
  git tag -f server-v0.9.0 >/dev/null
  table \"\$TABLE
    NewThing: new_thing() -> Thing;\"
  gated new_feature"
case_ gated_call_also_made_ungated "table \"\$TABLE
    NewThing: new_thing() -> Thing;\"
  proto new_feature
  gated new_feature
  echo 'fn g(c: RpcClient) { c.new_thing(); }' > src-tauri/src/commands/new.rs"
case_ gated_call_on_an_unknown_feature "table \"\$TABLE
    NewThing: new_thing() -> Thing;\"
  gated no_such_feature"

cd - >/dev/null
if [[ "${BLESS:-}" == "1" ]]; then
  cp "$RESULTS" "$GOLDEN"
  echo "→ blessed $GOLDEN"
elif diff -u "$GOLDEN" "$RESULTS"; then
  echo "✓ server compat fixtures match $GOLDEN ($(grep -c '^## ' "$RESULTS") cases)"
else
  echo "✗ results differ from the golden (above); BLESS=1 rewrites it after review"
  exit 1
fi
