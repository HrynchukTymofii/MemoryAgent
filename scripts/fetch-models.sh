#!/usr/bin/env bash
# Download the models the app needs. The macOS and Linux counterpart of
# fetch-models.ps1, with the same names and the same destinations.
#
#   ./scripts/fetch-models.sh               # base.en  (~148 MB, the default)
#   ./scripts/fetch-models.sh tiny.en       # ~78 MB, for the Light profile
#   ./scripts/fetch-models.sh small.en      # ~488 MB, better accuracy
#   ./scripts/fetch-models.sh embedding     # ~33 MB + the ONNX Runtime
#   ./scripts/fetch-models.sh router        # ~609 MB, the Tier 1 intent router
#   ./scripts/fetch-models.sh router-light  # ~378 MB, same model, Q4
#   ./scripts/fetch-models.sh onnxruntime   # ~17 MB, the runtime on its own
#   ./scripts/fetch-models.sh install       # put what is fetched where the app looks
#   ./scripts/fetch-models.sh bundle        # stage what the installer ships
#
# Weights are never committed — .gitignore excludes *.bin, *.onnx and friends.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
model="${1:-base.en}"

fail() { printf '\033[31m%s\033[0m\n' "$1" >&2; exit 1; }
ok()   { printf '\033[32m%s\033[0m\n' "$1"; }

# $1 url, $2 destination, $3 expected size in MB.
get_file() {
    local url="$1" dest="$2" expect="$3"
    if [ -f "$dest" ]; then
        ok "Already present: $dest ($(du -m "$dest" | cut -f1) MB)"
        return
    fi
    mkdir -p "$(dirname "$dest")"
    echo "Downloading $(basename "$dest") (about $expect MB)"
    echo "  from $url"
    echo "  to   $dest"

    # --fail so an HTML error page is never written out as if it were a model,
    # and a partial file is removed rather than left to be found later by the
    # app, which would load it and produce nothing but noise.
    if ! curl -L --fail --progress-bar -o "$dest" "$url"; then
        rm -f "$dest"
        fail "Download failed: $url"
    fi

    local have
    have="$(du -m "$dest" | cut -f1)"
    if [ "$have" -lt $(( expect / 2 )) ]; then
        rm -f "$dest"
        fail "Downloaded file is only $have MB, expected about $expect MB. Removed the partial file."
    fi
    ok "Done: $dest ($have MB)"
}

# The ONNX Runtime itself, which is a separate problem from the model.
#
# memos-embed loads it by path at run time (ORT_DYLIB_PATH) rather than linking
# it, so without this file semantic search is simply absent and the app falls
# back to keyword search — working, quietly worse, and with nothing on screen
# to say why. 1.20 is the floor the crate's `api-20` feature pins.
#
# universal2 rather than arm64: the shipped bundle is universal, and a runtime
# that only carries one architecture would make it half-broken on the other.
fetch_onnxruntime() {
    [ "$(uname)" = "Darwin" ] || fail "This fetches the macOS runtime; on Linux use the linux-x64 build."
    local dest="$root/runtime/libonnxruntime.dylib"
    if [ -f "$dest" ]; then
        ok "Already present: $dest"
        return
    fi
    local version="1.20.1"
    local url="https://github.com/microsoft/onnxruntime/releases/download/v$version/onnxruntime-osx-universal2-$version.tgz"
    local tmp
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' RETURN

    echo "Downloading ONNX Runtime $version (about 17 MB)"
    curl -L --fail --progress-bar -o "$tmp/ort.tgz" "$url" || fail "Download failed: $url"
    tar -xzf "$tmp/ort.tgz" -C "$tmp"

    # The archive ships the real file as libonnxruntime.$version.dylib with a
    # symlink beside it. Copy the target, not the link: the link is relative to
    # a directory that is about to be deleted.
    #
    # The .dSYM is excluded explicitly, and not as a tidiness measure: the
    # debug bundle contains a file of exactly the same name, `find` reaches it
    # first about as often as not, and it is a Mach-O of type dSYM rather than
    # a library — so the copy succeeds, the app starts, and dlopen fails at run
    # time with "unloadable mach-o file type 10".
    local lib
    lib="$(find "$tmp" -path "*.dSYM" -prune -o -name "libonnxruntime.*.dylib" -type f -print \
        | head -1)"
    [ -n "$lib" ] || fail "No libonnxruntime dylib inside the archive."
    file "$lib" | grep -q "dynamically linked shared library" \
        || fail "Found $lib but it is not a loadable library."
    mkdir -p "$root/runtime"
    cp "$lib" "$dest"

    # Ad-hoc signed because it is a downloaded binary that will be loaded by a
    # signed process under the hardened runtime, which refuses a library whose
    # signature it cannot evaluate.
    codesign --force --sign - "$dest" 2>/dev/null || true
    ok "Done: $dest"
}

# Where the installed app looks. The repository layout and the installed layout
# are different shapes, not the same shape in two places — under the data
# directory whisper sits at models/ggml-*.bin, with no stt/ level — so this
# maps between them rather than copying a tree.
#
# Hard links, because these files run to hundreds of megabytes and a second
# copy on the same volume is pure waste. A link also survives the repository
# being moved, which a symlink would not.
install_into_data_dir() {
    local data="$HOME/Library/Application Support/PersonalMemoryOS"
    [ "$(uname)" = "Darwin" ] || fail "Only the macOS layout is handled here."

    local placed=0
    place() {
        local src="$1" dst="$2"
        [ -f "$src" ] || return 0
        mkdir -p "$(dirname "$dst")"
        if [ -f "$dst" ]; then
            ok "Already installed: $dst"
        else
            ln "$src" "$dst" 2>/dev/null || cp "$src" "$dst"
            ok "Installed: $dst"
        fi
        placed=$((placed + 1))
    }

    for m in "$root"/models/stt/ggml-*.bin; do
        place "$m" "$data/models/$(basename "$m")"
    done
    place "$root/models/embedding/model.onnx"     "$data/models/embedding/model.onnx"
    place "$root/models/embedding/tokenizer.json" "$data/models/embedding/tokenizer.json"
    place "$root/models/llm/router.gguf"          "$data/models/llm/router.gguf"
    place "$root/runtime/libonnxruntime.dylib"    "$data/runtime/libonnxruntime.dylib"

    # The runtime is a library the app dlopens, not data, and the hardened
    # runtime enforces library validation: a process signed by a team may only
    # load code signed by that same team. A downloaded, ad-hoc-signed dylib is
    # refused with "different Team IDs", `ort` turns that refusal into a panic,
    # and `panic = "abort"` turns the panic into a dead application.
    #
    # So it is signed to match whatever signed the installed app — read from the
    # app rather than chosen here, because those two answers must agree and only
    # one of them is ours to pick.
    if [ -f "$data/runtime/libonnxruntime.dylib" ]; then
        local authority app="/Applications/PersonalMemoryOS.app"
        authority="$(codesign -dv --verbose=2 "$app" 2>&1 \
            | grep '^Authority=' | head -1 | cut -d= -f2-)"
        [ -n "$authority" ] || authority="-"
        codesign --force --sign "$authority" "$data/runtime/libonnxruntime.dylib" 2>/dev/null \
            && ok "Signed the ONNX Runtime to match the app ($authority)" \
            || fail "Could not sign the ONNX Runtime with '$authority'."
    fi

    if [ "$placed" -eq 0 ]; then
        fail "Nothing to install — fetch a model first."
    fi
    echo
    ok "Restart the app to pick these up."
}

# What goes inside the installer.
#
# The point of this target is that a person who installs the app never runs a
# script. tiny.en rather than base.en: 78 MB against 148 MB, and it is the floor
# that makes the product work on first launch, not the ceiling — base.en is
# offered later as an upgrade and wins over this one wherever both exist.
#
# The ONNX Runtime is shipped as well, and it is the one that is not optional to
# bundle. It is a library the process dlopens, and the hardened runtime only
# lets a signed process load code signed by the same team — so it has to be
# signed with the app, which happens only if it is inside the bundle when
# `tauri build` signs it. Everything the `install` target works around at run
# time stops being a problem here.
#
# The layout is the one the app looks for; see bundle_dir() in main.rs.
stage_for_bundle() {
    "$0" tiny.en
    "$0" embedding

    local staging="$root/apps/desktop/src-tauri/resources"
    mkdir -p "$staging/models/embedding" "$staging/runtime"

    stage() {
        local src="$root/$1" dst="$staging/$2"
        [ -f "$src" ] || fail "Missing $src — fetch it first."
        # Copied, not linked: this file is about to be signed as part of the
        # application, and signing follows a hard link back to the original.
        cp -f "$src" "$dst"
        ok "Staged: $2 ($(du -m "$dst" | cut -f1) MB)"
    }

    stage "models/stt/ggml-tiny.en.bin"      "models/ggml-tiny.en.bin"
    stage "models/embedding/model.onnx"      "models/embedding/model.onnx"
    stage "models/embedding/tokenizer.json"  "models/embedding/tokenizer.json"
    # Only macOS has a fetch path for the runtime, and macOS is the platform
    # where bundling it is load-bearing rather than a convenience. Elsewhere the
    # app finds it beside the executable as it always has.
    if [ "$(uname)" = "Darwin" ]; then
        stage "runtime/libonnxruntime.dylib" "runtime/libonnxruntime.dylib"
    fi

    echo
    ok "Staged $(du -sm "$staging" | cut -f1) MB. ./scripts/build-macos.sh now ships it."
}

case "$model" in
  # Fetching leaves everything in the repository, which is where a `cargo run`
  # looks and where the installed app never does. Without this step the app in
  # /Applications reports every model missing no matter how many were fetched.
  install)
    install_into_data_dir
    exit 0
    ;;

  bundle)
    stage_for_bundle
    exit 0
    ;;

  onnxruntime)
    fetch_onnxruntime
    exit 0
    ;;

  # bge-small-en-v1.5, int8-quantized ONNX export. Quantized, at a third of the
  # size, because it runs on a background worker while somebody keeps working.
  # Both files must come from the same export — a tokenizer from another
  # revision produces ids the graph never saw, and that is not an error, it is
  # quietly worse search.
  embedding)
    dir="$root/models/embedding"
    base="https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main"
    get_file "$base/onnx/model_quantized.onnx" "$dir/model.onnx" 33
    get_file "$base/tokenizer.json" "$dir/tokenizer.json" 1
    # The model is useless without the runtime that executes it, so this is one
    # command rather than two that must be discovered in the right order.
    [ "$(uname)" = "Darwin" ] && fetch_onnxruntime
    echo
    ok "Embedding model ready. Check it with:"
    echo "  cargo run -p memos-embed --features onnx --example embed_check"
    exit 0
    ;;

  # The Tier 1 router (ADR-0003). Qwen3 0.6B at Q8 rather than Q4: this model
  # decodes under a GBNF grammar, so it is mostly choosing among tokens the
  # grammar already allows, and what is left to get wrong is the intent decision
  # itself — exactly what heavy quantisation costs you at this size.
  router|router-light)
    dir="$root/models/llm"
    if [ "$model" = "router" ]; then
        url="https://huggingface.co/Qwen/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q8_0.gguf"
        mb=609
    else
        url="https://huggingface.co/unsloth/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q4_K_M.gguf"
        mb=378
    fi
    # One filename whatever the quantisation, so the app finds the router
    # without being told which build the user chose.
    get_file "$url" "$dir/router.gguf" "$mb"
    echo
    ok "Router model ready. Build the sidecar with:"
    echo "  ./scripts/build-router.sh"
    exit 0
    ;;

  tiny.en)   mb=78   ;;
  base.en)   mb=148  ;;
  small.en)  mb=488  ;;
  medium.en) mb=1533 ;;
  *)
    fail "Unknown model '$model'. Choose one of: tiny.en, base.en, small.en, medium.en, embedding, router, router-light, onnxruntime, install, bundle"
    ;;
esac

get_file "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-$model.bin" \
         "$root/models/stt/ggml-$model.bin" "$mb"
