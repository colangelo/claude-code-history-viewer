#!/bin/bash
# cchv-launch.sh — bao-first secret resolution wrapper for the cchv archive
# launchd jobs (house deployment; see docs/archive/deployment.md).
#
# Usage: cchv-launch.sh <daemon|hub> [--render-only]
#
# Resolution order for secrets:
#   1. OpenBao via the `cchv-daemon` AppRole (creds in ~/.config/cchv/bao-approle)
#   2. `op read` from 1Password (vault AC-DevOps) — may require Touch ID
#   3. last-known-good rendered config from a previous successful launch
#
# The static configs (~/.config/cchv/daemon.toml, hub.toml) are TEMPLATES with
# @PLACEHOLDER@ markers — no secrets on disk outside the 0600 runtime renders.
#
# AppRole creds file format (~/.config/cchv/bao-approle, chmod 0600):
#   role_id=...
#   secret_id=...
# Materialize once per machine from 1P item `openbao - cchv-daemon approle`.

set -euo pipefail
export PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"

BAO_ADDR="${BAO_ADDR:-https://secrets.cat-bluegill.ts.net}"
CFG_DIR="$HOME/.config/cchv"
APPROLE_FILE="$CFG_DIR/bao-approle"
HOST="$(hostname -s)"

MODE="${1:-}"
RENDER_ONLY="${2:-}"
case "$MODE" in
  daemon|hub) ;;
  *) echo "usage: $0 <daemon|hub> [--render-only]" >&2; exit 2 ;;
esac

TEMPLATE="$CFG_DIR/$MODE.toml"
RUNTIME="$CFG_DIR/$MODE.runtime.toml"

log() { echo "[cchv-launch:$MODE] $*" >&2; }

# --- OpenBao: AppRole login, then kv reads ---------------------------------
BAO_TOKEN=""
bao_login() {
  [ -r "$APPROLE_FILE" ] || { log "no AppRole creds at $APPROLE_FILE"; return 1; }
  local role_id secret_id
  role_id="$(sed -n 's/^role_id=//p' "$APPROLE_FILE")"
  secret_id="$(sed -n 's/^secret_id=//p' "$APPROLE_FILE")"
  [ -n "$role_id" ] && [ -n "$secret_id" ] || { log "AppRole creds file malformed"; return 1; }
  BAO_TOKEN="$(curl -sf --max-time 10 "$BAO_ADDR/v1/auth/approle/login" \
    -d "{\"role_id\":\"$role_id\",\"secret_id\":\"$secret_id\"}" \
    | jq -r '.auth.client_token // empty')"
  [ -n "$BAO_TOKEN" ] || { log "AppRole login failed against $BAO_ADDR"; return 1; }
}

# bao_kv <secret-name> <field>  → value on stdout, or fail
bao_kv() {
  [ -n "$BAO_TOKEN" ] || return 1
  local val
  val="$(curl -sf --max-time 10 -H "X-Vault-Token: $BAO_TOKEN" \
    "$BAO_ADDR/v1/kv/data/infra/cchv/$1" | jq -r --arg f "$2" '.data.data[$f] // empty')"
  [ -n "$val" ] && echo "$val"
}

# op_read <op://ref>  → value on stdout, or fail (Touch ID may prompt/stall)
op_read() {
  local val
  val="$(op read "$1" 2>/dev/null)" || return 1
  [ -n "$val" ] && echo "$val"
}

# resolve <bao-secret> <bao-field> <op-ref>  → value, bao-first then op
resolve() {
  local val
  if val="$(bao_kv "$1" "$2")"; then echo "$val"; return 0; fi
  log "bao read $1/$2 failed — falling back to op read"
  if val="$(op_read "$3")"; then echo "$val"; return 0; fi
  return 1
}

# --- render -----------------------------------------------------------------
render() {
  [ -r "$TEMPLATE" ] || { log "missing template $TEMPLATE"; return 1; }
  local content
  content="$(<"$TEMPLATE")"
  bao_login || true   # op fallback still possible without a bao token

  case "$MODE" in
    daemon)
      local hub_token
      hub_token="$(resolve hub-tokens "${HOST}_token" \
        "op://AC-DevOps/cchv - archive hub tokens/${HOST} token")" || return 1
      content="${content//@HUB_TOKEN@/$hub_token}"
      ;;
    hub)
      local db_pass m4m_token mbm5_token
      db_pass="$(resolve pg1 password \
        "op://AC-DevOps/cchv - app role @ pg1/password")" || return 1
      m4m_token="$(resolve hub-tokens m4m_token \
        "op://AC-DevOps/cchv - archive hub tokens/m4m token")" || return 1
      mbm5_token="$(resolve hub-tokens ac-mbm5_token \
        "op://AC-DevOps/cchv - archive hub tokens/ac-mbm5 token")" || return 1
      content="${content//@DB_PASSWORD@/$db_pass}"
      content="${content//@M4M_TOKEN@/$m4m_token}"
      content="${content//@AC_MBM5_TOKEN@/$mbm5_token}"
      ;;
  esac

  if printf '%s\n' "$content" | grep -q '@[A-Z_]*@'; then
    log "unresolved placeholders remain after render"; return 1
  fi
  ( umask 077; printf '%s\n' "$content" > "$RUNTIME.tmp" )
  mv -f "$RUNTIME.tmp" "$RUNTIME"
  log "rendered $RUNTIME (bao-first)"
}

if ! render; then
  if [ -s "$RUNTIME" ]; then
    log "WARN: secret resolution failed — reusing last-known-good $RUNTIME"
  else
    log "FATAL: secret resolution failed and no cached $RUNTIME exists"
    exit 1
  fi
fi

[ "$RENDER_ONLY" = "--render-only" ] && exit 0

case "$MODE" in
  daemon) export DAEMON_CONFIG="$RUNTIME"; exec "$HOME/.local/bin/cchv-sync-daemon" ;;
  hub)    export HUB_CONFIG="$RUNTIME";    exec "$HOME/.local/bin/cchv-hub" ;;
esac
