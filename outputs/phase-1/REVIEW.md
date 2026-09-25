# Review — Phase 1: Userspace runtime library & build integration

**Date:** 2026-09-24
**Files reviewed:** 8 (`userspace/Cargo.toml`, `userspace/src/lib.rs`, `userspace/src/bin/hello.rs`, `userspace/build.sh`, `userspace/.cargo/config.toml`, `userspace/ABI.md` + kernel cross-checks in `kernel/src/syscall.rs`, `kernel/src/process.rs`, `kernel/src/boot.S`)
**Result:** ⚠️ APPROVED WITH OBSERVATIONS

## Plan Compliance

| Expected output | Status | Observation |
|-----------------|--------|-------------|
| `userspace/Cargo.toml` (standalone no_std manifest) | ✅ Compliant | Excludes crate from workspace; `[lib]` + `[[bin]] hello`. Correct. |
| `userspace/src/lib.rs` (wrappers + ipc + `_start` + panic) | ✅ Compliant | Wrappers ported verbatim from kernel; IpcMessage byte-for-byte match; `_start` sets RSP=0xFFF000 → main; panic handler spins (matches kernel). |
| `userspace/ABI.md` (load base, entry, registers, IpcMessage) | ✅ Compliant | Documents 0x400000 / `_start` / rax-rdi…r8 convention / IpcMessage. Good single source of truth. |
| Build wiring → ET_EXEC @ 0x400000 | ✅ Compliant | Verified: clean rebuild exit 0, `readelf -h` → EXEC, entry 0x4000a0 = `_start`. |

**Gate criteria (phase-1.md):**
- [x] Compiles to ELF-64 ET_EXEC with entry point in the 0x400000 region — ✅ verified (`readelf -h` → `Type: EXEC`, `Entry point: 0x4000a0`).
- [x] Syscall wrappers match kernel SYSCALL convention (verified against boot.S) — ✅ verified against `syscall_entry` LSTAR path (syscall.rs:793-800): `rax=nr, rdi=a1, rsi=a2, rdx=a3, r10=a4, r8=a5`.
- [x] hello-world program builds and inspected; no address collision — ✅ verified. Load base 0x400000 avoids kernel (0x100000) and user stack (0xFFF000).

## Issues Found

### 🔴 Critical (block next phase)
- None.

### 🟡 Important (should fix, does not block)
- **`userspace/.cargo/config.toml` + `userspace/Cargo.toml:21` + `lib.rs:8-9` — internal inconsistency / dead config.** The canonical build path (`build.sh`) runs cargo from a scratch dir (`mktemp -d`) and passes every flag via command-line `--config`; it therefore **never loads** `userspace/.cargo/config.toml`. Yet that file's header says *"Each target inherits its own flags from this crate's `.cargo/config.toml`"* and `Cargo.toml:21` repeats it. This contradicts `build.sh` (lines 11-18) and `DECISIONS.md`, and building directly (`cd userspace && cargo build`) actually **fails** because the repo-root `.cargo/config.toml` leaks `-Tkernel/src/linker.ld`. The file provides no functional value on the working path and its comments are misleading. Recommend deleting it (or at minimum correcting its header to state it is intentionally unused by `build.sh`).

### 🔵 Minor (improvement suggestions)
- **`userspace/build.sh:40-41` — release-mode output path is wrong.** `${RELEASE:+release}` expands to `release` when `RELEASE=" --release"`, producing the non-existent dir `…/releasedebug/deps/`. Debug mode works (`…/debug/deps/`), but `build.sh hello release` prints a bogus path. Fix: use `${RELEASE:+release}/` vs `/debug` branch, or strip the trailing ` --release` before interpolating. The ELF itself builds correctly regardless — this only affects the printed `Built:` path.
- **`userspace/src/lib.rs:8-9` — doc references a nonexistent `examples/` dir.** Comment says *"separate bin targets (see `examples/`)"*, but `hello` is defined as `[[bin]]` at `src/bin/hello.rs`, not under `examples/`. Cosmetic doc inaccuracy.

## Observations (not blocking, for awareness)
- **Scope: implemented `syscall0..3`, phase text says `syscall0..5`.** The builder documented the rationale (`lib.rs:24`: *"The kernel only implements syscall0..syscall3; we port exactly those"*). This is justified — the kernel source genuinely defines only 0..3 (verified at syscall.rs:1419-1470) and no inline server in `user_program.rs` uses 4/5. The wrappers are trivially extensible if a future syscall needs ≥4 args (`syscall4` → r10=a4, `syscall5` → r8=a5). Noted for phase 2/3 awareness.
- **Intentional deviations documented** — the build-from-scratch-dir strategy (DECISIONS.md) and the PIE-vs-ET_EXEC root cause are both recorded in `SUMMARY.md`. No false positives to correct.

## Summary
- 🔴 Critical: 0
- 🟡 Important: 1
- 🔵 Minor: 2
- ✅ Plan compliance: 100% (all four expected outputs present and meet spec)
- 📊 Overall quality: 9/10 — clean, well-documented, ABI matches the kernel verbatim, includes a compile-time `IpcMessage` size assertion. The only real blemish is the dead/misleading `.cargo/config.toml`.

## Final Verdict

⚠️ **APPROVED WITH OBSERVATIONS** — 0 critical; 1 important (internal inconsistency in build config) + 2 minor. Phase works and all gate criteria pass with fresh evidence (`cargo build` exit 0, `readelf -h` → ET_EXEC @ entry 0x4000a0 = `_start`). The important item should be cleaned up before the next phase so phase-2/3 builders are not misled, but it does not block continuation.

---
### Gate evidence (independently re-run this review)
```
$ cargo +nightly clean && ./userspace/build.sh hello
   Compiling core ... Compiling userspace v0.1.0 ... Finished `dev` profile ...
   Built: /home/juan/Repos/LearnOS/userspace/target/x86_64-unknown-none/debug/deps/hello-d1429c9be38eb299

$ readelf -h target/x86_64-unknown-none/debug/deps/hello-d1429c9be38eb299
  Tipo:                              EXEC (Fichero ejecutable)
  Máquina:                           Advanced Micro Devices X86-64
  Dirección del punto de entrada:    0x4000a0

$ readelf -sW target/x86_64-unknown-none/debug/deps/hello-d1429c9be38eb299 | grep _start
    23: 00000000004000a0    17 FUNC    GLOBAL DEFAULT    1 _start
```
