#!/bin/sh
# Builds the browser viewer (crates/stave-web) into one self-contained
# page, target/viewer/stave-viewer.html: the WASM (as base64) and its
# wasm-bindgen glue inlined, so the page needs no other origin. The MCP
# server shows it as its MCP App page and serves it on localhost.
#
#   tools/build-viewer.sh
#
# Needs the wasm32-unknown-unknown target and the wasm-bindgen CLI of the
# same version as the wasm-bindgen crate in Cargo.lock. It installs
# nothing: when either is missing it says how to get it and stops.
#
# wasm-opt (binaryen) should be on PATH too: it shrinks the page from about
# 3.6 MB to about 3.2 MB. Without it the build still works, with a warning.
# The WASM is built with the `viewer` profile (Cargo.toml: size-tuned,
# LTO) and wasm-bindgen drops its name section; the page assembler is
# built with `--release`.
#
# CARGO_TARGET_DIR is honoured for the build; the page always goes to
# target/viewer/ in the repository.
set -eu

cd "$(dirname "$0")/.."
root=$(pwd)
target_dir=${CARGO_TARGET_DIR:-$root/target}
out_dir=$root/target/viewer
work=$target_dir/viewer-build

# The wasm-bindgen version the crate was built with, from Cargo.lock.
wanted=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)
if [ -z "$wanted" ]; then
    echo "build-viewer: Cargo.lock names no wasm-bindgen version" >&2
    exit 1
fi

if ! command -v wasm-bindgen >/dev/null 2>&1; then
    echo "build-viewer: the wasm-bindgen CLI is missing." >&2
    echo "  Install the version that matches Cargo.lock:" >&2
    echo "    cargo install wasm-bindgen-cli --version $wanted --locked" >&2
    exit 1
fi
have=$(wasm-bindgen --version | awk '{ print $2 }')
if [ "$have" != "$wanted" ]; then
    echo "build-viewer: wasm-bindgen CLI $have is installed, but Cargo.lock has $wanted." >&2
    echo "  Install the matching one:" >&2
    echo "    cargo install wasm-bindgen-cli --version $wanted --locked" >&2
    exit 1
fi

if command -v rustup >/dev/null 2>&1 &&
    ! rustup target list --installed | grep -qx wasm32-unknown-unknown; then
    echo "build-viewer: the wasm32-unknown-unknown target is missing." >&2
    echo "  Add it with: rustup target add wasm32-unknown-unknown" >&2
    exit 1
fi

cargo build --quiet --profile viewer -p stave-web --lib --target wasm32-unknown-unknown
cargo build --quiet --release -p stave-web --bin stave-viewer-page

rm -rf "$work"
mkdir -p "$work" "$out_dir"
wasm-bindgen --target no-modules --no-typescript --remove-name-section \
    --remove-producers-section --out-dir "$work" \
    "$target_dir/wasm32-unknown-unknown/viewer/stave_web.wasm"

module=$work/stave_web_bg.wasm
# The features enabled are the ones rustc's wasm32 target emits and the
# module uses (wasm-bindgen's externref table needs reference types).
# wasm-opt also reads them from the module's target_features section;
# naming them here keeps it from rejecting or lowering them if that
# section is ever missing. A failing wasm-opt stops the build.
if command -v wasm-opt >/dev/null 2>&1; then
    wasm-opt -Oz \
        --enable-bulk-memory --enable-nontrapping-float-to-int \
        --enable-sign-ext --enable-mutable-globals \
        --enable-reference-types --enable-multivalue \
        -o "$work/stave_web_opt.wasm" "$module"
    module=$work/stave_web_opt.wasm
else
    echo "build-viewer: WARNING: wasm-opt (binaryen) is not on PATH, so the" >&2
    echo "  module is not optimised and the page is about 0.4 MB larger." >&2
    echo "  Install binaryen (Arch: pacman -S binaryen; Debian/Ubuntu: apt" >&2
    echo "  install binaryen; or a release from" >&2
    echo "  https://github.com/WebAssembly/binaryen/releases) and run again." >&2
fi

"$target_dir/release/stave-viewer-page" "$work/stave_web.js" "$module" \
    "$out_dir/stave-viewer.html"

echo "build-viewer: wrote target/viewer/stave-viewer.html ($(wc -c <"$out_dir/stave-viewer.html") bytes)"
