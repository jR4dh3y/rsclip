#!/usr/bin/env bash
# Explicit setup only: OCR itself never makes network requests.
set -euo pipefail

revision=ed345c7f158eddbc7dee0c6a7269afb75a94f7e8
model_dir="${1:-${INDRA_OCR_MODEL_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/rsclip/indra/$revision}}"
base="https://huggingface.co/datalab-to/indra-ocr/resolve/$revision"

command -v curl >/dev/null
command -v sha256sum >/dev/null
mkdir -p "$model_dir"
staging=$(mktemp -d "$model_dir/.download.XXXXXXXX")
trap 'rm -rf "$staging"' EXIT

# Checksums from the pinned Hugging Face revision, never a floating tag.
while read -r hash name; do
    if [[ -f "$model_dir/$name" ]] && printf '%s  %s\n' "$hash" "$model_dir/$name" | sha256sum --check --status; then
        continue
    fi
    printf 'Downloading %s\n' "$name" >&2
    curl --fail --location --retry 3 --connect-timeout 15 --max-time 300 \
        "$base/$name" --output "$staging/$name"
    printf '%s  %s\n' "$hash" "$staging/$name" | sha256sum --check --status
done <<'MODELS'
46b0050ce4c4692f887a1a9c038480223af1c7a91b3fcced2657d9cf00290dc4 det.onnx
e243be069b357e56a0e867a21d7482e498df42fb19678bfb67a25f48b1f06151 rec.onnx
23635dd2324b4ccc00c408af961273a2dedd2e4f0386bc54dba678c41bcf6b46 order.onnx
8d7bfce5d08bcad55955f41ca494a7f9d9d3ed5d5b6da1f6f276d5c509590729 orient.onnx
5ba37030e817affede0097a7503b6829624104166c6e0b6aacecacf919fc7d11 script.onnx
4cec840b55e1bb7f375daf30a2975404a867a71ac1a5bc5b1629c4909ce16baa rec_cjk.onnx
ff6ec58f91c93a475ae9505ad21c73c3ecda0b378f1d889dd83408e10832d1bb meta.json
50c88a018a7ee322e89f6b09401b237beb0cd331685879405b3db48993950a5f LICENSE
MODELS

# Publish only after every downloaded file passes verification.
for file in "$staging"/*; do
    [[ -f "$file" ]] && mv -- "$file" "$model_dir/"
done
printf 'Indra models ready in %s\n' "$model_dir"
