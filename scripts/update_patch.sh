#!/usr/bin/env bash
# Build an optional patch from the previously published Linux binary.
set -euo pipefail

current=$1
assets=$2
previous_json=$(gh api "repos/$GITHUB_REPOSITORY/releases/latest")
previous=$(jq -er '.tag_name' <<<"$previous_json")
if [[ ! "$previous" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ || "$previous" == "$GITHUB_REF_NAME" ]]; then
    exit 0
fi

read -r url digest < <(jq -er '.assets[] | select(.name == "cibitube") | [.browser_download_url, .digest] | @tsv' <<<"$previous_json")
if [[ "$url" != "https://github.com/$GITHUB_REPOSITORY/releases/download/$previous/cibitube" || ! "$digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
    echo 'Previous Linux release does not have verifiable metadata.'
    exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
curl --fail --silent --show-error --location --max-time 90 --output "$tmp/old" "$url"
base_hash=${digest#sha256:}
printf '%s  %s\n' "$base_hash" "$tmp/old" | sha256sum --check --status
zstd -q --patch-from "$tmp/old" "$current" -o "$tmp/patch.zst"
if (( $(wc -c < "$tmp/patch.zst") * 100 < $(wc -c < "$current") * 85 )); then
    zstd -q -d --patch-from "$tmp/old" "$tmp/patch.zst" -o "$tmp/rebuilt"
    cmp "$tmp/rebuilt" "$current"
    mv "$tmp/patch.zst" "$assets/cibitube-linux-from-$previous-$base_hash.zst"
fi
