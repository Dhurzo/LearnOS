#!/usr/bin/env bash
# Build the LearnOS user-space interrupt server as a non-PIE ET_EXEC loaded at
# 0x400000, so the kernel can embed it and load it at boot via `load_elf`.
#
# Usage:
#   ./build.sh [release] [--out DIR]
# Examples:
#   ./build.sh                 # build in debug mode
#   ./build.sh release         # build in release mode
#   ./build.sh --out <dir>     # copy the resulting ELF into <dir> as interrupt_server.elf
#
# Why the scratch-dir + --config pattern (same as userspace/build.sh):
#   Cargo loads file-based .cargo/config.toml relative to the CWD and its
#   ancestors, NOT the manifest's own directory. The repo root ships a
#   .cargo/config.toml that injects the KERNEL linker script
#   (`-Tkernel/src/linker.ld`) into every cargo invocation under it -- and when
#   kernel/build.rs runs this from inside the tree, that leaked flag would reach
#   this link step and fail. We sidestep all of this by running in a scratch
#   dir outside the repo and passing flags via `--config`, which replaces the
#   default config search entirely; no ancestor config can leak in regardless of
#   CWD. No `-Z build-std` is needed: x86_64-unknown-none ships a prebuilt core
#   in the toolchain, exactly as `userspace/build.sh` relies on.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

BIN="interrupt_server"
RELEASE=""
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
BIN="interrupt_server"
[[ "${positional[0]:-}" == "release" ]] && RELEASE=" --release"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Clear any inherited rustflags (the root workspace injects the kernel linker
# script into CARGO_ENCODED_RUSTFLAGS when invoked from build.rs).
unset CARGO_ENCODED_RUSTFLAGS RUSTFLAGS

(
  cd "$WORK"
  cargo +nightly build \
    --manifest-path "$HERE/Cargo.toml" \
    --bin "$BIN" \
    --target x86_64-unknown-none \
    --config "build.rustflags=[\"-C\",\"link-arg=-Ttext=0x400000\",\"-C\",\"link-arg=-no-pie\",\"-C\",\"link-arg=-e\",\"-C\",\"link-arg=_start\"]" \
    $RELEASE
)

OUT="$(ls "$HERE"/target/x86_64-unknown-none/${RELEASE:+release}debug/deps/$BIN-* 2>/dev/null | grep -vE '\.(d|o)$' | head -1)"
[[ -z "$OUT" ]] && OUT="$HERE/target/x86_64-unknown-none/${RELEASE:+release}debug/deps/$BIN"

if [[ -n "$OUT_DIR" ]]; then
  mkdir -p "$OUT_DIR"
  cp -f "$OUT" "$OUT_DIR/interrupt_server.elf"
  echo "Copied $BIN ELF -> $OUT_DIR/interrupt_server.elf"
else
  echo "Built: $OUT"
fi
