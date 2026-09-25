#!/usr/bin/env bash
# Build a LearnOS user-space image as a non-PIE ET_EXEC loaded at 0x10000000.
#
# Usage:
#   ./build.sh [bin-name] [release] [--out DIR]
# Examples:
#   ./build.sh            # build the `hello` bin in debug mode
#   ./build.sh hello      # same (explicit)
#   ./build.sh hello release
#   ./build.sh vga --out kernel/src/server_elfs
#     builds the `vga` image and copies the resulting ELF into the given dir.
#
# Why --config-path (hermetic build):
#   Cargo loads file-based .cargo/config.toml relative to the CWD and its
#   ancestors, NOT the manifest's own directory. The repo root ships a
#   .cargo/config.toml that injects the KERNEL linker script (`-Tkernel/src/
#   linker.ld`) into every cargo invocation under it -- and when cargo runs
#   build.sh from inside `kernel/build.rs`, that leaked flag reaches the
#   userspace link step and fails. We sidestep all of this by writing our own
#   config to a scratch file and pointing cargo at it with `--config-path`,
#   which replaces the default config search entirely; every needed flag lives
#   in that single file, so no ancestor config can leak in regardless of CWD.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Parse args flexibly: first non-flag positional is the bin name, optional
# second positional is "release", and "--out DIR" may appear anywhere.
BIN="hello"
RELEASE=0
OUT_DIR=""
positional=()
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
  if [[ "${args[$i]}" == "--out" ]]; then
    OUT_DIR="${args[$((i + 1))]}"
    i=$((i + 1))
  elif [[ "${args[$i]}" != --* ]]; then
    positional+=("${args[$i]}")
  fi
done
BIN="${positional[0]:-hello}"
[[ "${positional[1]:-}" == "release" ]] && RELEASE=1

CARGO_PROFILE=debug
CARGO_RELEASE_ARGS=()
if [[ "$RELEASE" == "1" ]]; then
  CARGO_PROFILE=release
  CARGO_RELEASE_ARGS=(--release)
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Cargo sets CARGO_ENCODED_RUSTFLAGS for build scripts (inheriting the repo
# root's kernel linker script `-Tkernel/src/linker.ld`). If we did not strip it,
# this leaked flag would reach every userspace link step and fail. Clear any
# inherited rustflags so only the flags below (via --config) apply.
unset CARGO_ENCODED_RUSTFLAGS RUSTFLAGS

(
  cd "$WORK"
  cargo +nightly build \
    --manifest-path "$HERE/Cargo.toml" \
    --bin "$BIN" \
    -Z build-std=core,compiler_builtins \
    --target x86_64-unknown-none \
    --config "build.rustflags=[\"-C\",\"link-arg=--image-base=0x10000000\",\"-C\",\"link-arg=-no-pie\",\"-C\",\"link-arg=-e\",\"-C\",\"link-arg=_start\"]" \
    "${CARGO_RELEASE_ARGS[@]}"
)

# Cargo refreshes the top-level executable for the selected profile. Avoid
# globbing `deps/`: old hashed executables can remain there after linker flags
# change, and selecting one can silently embed a stale VMA.
OUT="$HERE/target/x86_64-unknown-none/$CARGO_PROFILE/$BIN"

if [[ -n "$OUT_DIR" ]]; then
  mkdir -p "$OUT_DIR"
  cp -f "$OUT" "$OUT_DIR/$BIN.elf"
  echo "Copied $BIN ELF -> $OUT_DIR/$BIN.elf"
else
  echo "Built: $OUT"
fi
