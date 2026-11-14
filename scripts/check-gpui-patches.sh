#!/usr/bin/env bash
# Fails when a [patch.crates-io] fork stops being used.
#
# Cargo only warns when a patch entry no longer matches a dependency, which is
# easy to miss after an upstream version bump. Check the resolved package
# sources against the revs pinned in Cargo.toml.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
manifest="$repo_root/Cargo.toml"
cd "$repo_root"

# Read the pinned rev for one [patch.crates-io] entry so Cargo.toml stays the
# single source of truth.
manifest_rev() {
    local crate="$1"
    local rev
    rev=$(awk -v crate="$crate" '
        $1 == crate && match($0, /rev = "[0-9a-f]+"/) {
            print substr($0, RSTART + 7, RLENGTH - 8)
            exit
        }
    ' "$manifest")
    if [[ -z "$rev" ]]; then
        echo "::error::$crate is missing a git rev in Cargo.toml"
        exit 1
    fi
    echo "$rev"
}

metadata=$(cargo metadata --format-version 1 --locked)

check_patch() {
    local crate="$1"
    local repo="$2"
    local rev="$3"
    local source

    source=$(
        jq -r --arg crate "$crate" \
            '.packages[] | select(.name == $crate) | .source // empty' <<<"$metadata"
    )
    if [[ -z "$source" ]]; then
        echo "::error::$crate is missing from cargo metadata"
        exit 1
    fi
    if [[ "$source" != "git+https://github.com/${repo}?rev=${rev}#"* ]]; then
        echo "::error::$crate must resolve to ${repo}@${rev}, got: $source"
        exit 1
    fi
    echo "$crate -> $source"
}

gpui_component_rev=$(manifest_rev gpui-component)
gpui_fast_rev=$(manifest_rev gpui-fast)
gpui_fast_wgpu_rev=$(manifest_rev gpui-fast-wgpu)
gpui_fast_windows_rev=$(manifest_rev gpui-fast-windows)

check_patch gpui-component TommyZ-7/gpui-kit "$gpui_component_rev"
check_patch gpui-fast TommyZ-7/gpui-fast "$gpui_fast_rev"
check_patch gpui-fast-wgpu TommyZ-7/gpui-fast "$gpui_fast_wgpu_rev"
check_patch gpui-fast-windows TommyZ-7/gpui-fast "$gpui_fast_windows_rev"
