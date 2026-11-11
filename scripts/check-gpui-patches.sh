#!/usr/bin/env bash
# Fails when a [patch.crates-io] fork stops being used.
#
# Cargo only warns when a patch entry no longer matches a dependency, which is
# easy to miss after an upstream version bump. Check the resolved package
# sources instead of trusting the manifest.
set -euo pipefail

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

check_patch gpui-component TommyZ-7/gpui-kit ee98b887e620e7c47e0cbe41500959fde7bfdfb5
check_patch gpui-fast TommyZ-7/gpui-fast a3c3ac550d16bfb70e9f55c2cd46bf9d68cd4b73
check_patch gpui-fast-wgpu TommyZ-7/gpui-fast a3c3ac550d16bfb70e9f55c2cd46bf9d68cd4b73
check_patch gpui-fast-windows TommyZ-7/gpui-fast a3c3ac550d16bfb70e9f55c2cd46bf9d68cd4b73
