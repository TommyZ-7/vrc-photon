#!/usr/bin/env bash
# Fails when version-bearing documentation drifts from the source constants.
#
# The values live in Rust source; the user-facing prose repeats them. This
# check keeps the two sides in sync without requiring a test binary or a GPUI
# build, so it can run in the lightweight `changes` CI job too.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

fail() {
    echo "docs drift: $*" >&2
    exit 1
}

read_const() {
    local file=$1
    local pattern=$2
    local name=$3
    local value

    value=$(sed -n "$pattern" "$file" | head -n 1)
    if [ -z "$value" ]; then
        fail "cannot read ${name} from ${file}"
    fi
    printf '%s' "$value"
}

protocol=$(read_const \
    crates/ipc/src/lib.rs \
    's/^pub const PROTOCOL_VERSION: u32 = \([0-9][0-9]*\);$/\1/p' \
    PROTOCOL_VERSION)
schema=$(read_const \
    crates/core/src/db/schema.rs \
    's/^.*const SCHEMA_VERSION: i64 = \([0-9][0-9]*\);$/\1/p' \
    SCHEMA_VERSION)
thumb_entries=$(read_const \
    crates/ui/src/state/thumbnail.rs \
    's/^const THUMB_CACHE_ENTRIES: usize = \([0-9][0-9]*\);$/\1/p' \
    THUMB_CACHE_ENTRIES)
thumb_in_flight=$(read_const \
    crates/ui/src/state/thumbnail.rs \
    's/^const THUMB_IN_FLIGHT: usize = \([0-9][0-9]*\);$/\1/p' \
    THUMB_IN_FLIGHT)

grep -Fq "PROTOCOL_VERSION = ${protocol}" docs/architecture.md \
    || fail "docs/architecture.md must say PROTOCOL_VERSION = ${protocol}"
grep -Fq "PROTOCOL_VERSION = ${protocol}" docs/features.md \
    || fail "docs/features.md must say PROTOCOL_VERSION = ${protocol}"

grep -Fq "${thumb_entries}件のLRU" docs/architecture.md \
    || fail "docs/architecture.md must say ${thumb_entries}件のLRU"
grep -Fq "${thumb_entries}件LRU" docs/features.md \
    || fail "docs/features.md must say ${thumb_entries}件LRU"
grep -Fq "同時デコードは最大${thumb_in_flight}件" docs/architecture.md \
    || fail "docs/architecture.md must say 同時デコードは最大${thumb_in_flight}件"

grep -Fq "現在のバージョンは **${schema}**" docs/data-model.md \
    || fail "docs/data-model.md must say 現在のバージョンは **${schema}**"
grep -Fq "\`user_version\` が $((schema + 1)) 以上" docs/data-model.md \
    || fail "docs/data-model.md must reject user_version $((schema + 1)) 以上"
grep -Fq "最大値（${schema}）" docs/data-model.md \
    || fail "docs/data-model.md must say 最大値（${schema}）"

# ai-host gained a core dependency for the shared AI types. Keep the
# dependency diagram honest when that changes.
if grep -Fq 'vrc-photon-core' crates/ai-host/Cargo.toml; then
    grep -Fq 'ai-host ──> core' docs/architecture.md \
        || fail "docs/architecture.md must show ai-host ──> core"
else
    if grep -Fq 'ai-host ──> core' docs/architecture.md; then
        fail "docs/architecture.md still shows ai-host ──> core"
    fi
fi

echo "docs drift check OK (PROTOCOL_VERSION=${protocol}, SCHEMA_VERSION=${schema}, thumbnails=${thumb_entries}/${thumb_in_flight})"
