//! Embedded user-space server images (Phase 2 — ELF loader migration).
//!
//! Each LearnOS server (`vga`, `keyboard`, `init`, `shell`) used to be a Rust
//! function compiled *into* the kernel image. They are now standalone user-space
//! programs that each produce their own ET_EXEC ELF at load base `0x10000000`.
//!
//! The kernel build script (`build.rs`) compiles those images into
//! `$OUT_DIR/server_elfs/<name>.elf` and generates the byte-array constants seen
//! below. The loader reads them from here (via `include_bytes!`) instead of
//! resolving inline function addresses — this is what makes the servers true
//! user processes rather than kernel code.

// Textually paste the generated constants into this module so they are visible
// as `crate::server_images::<NAME>_ELF`.
include!(concat!(env!("OUT_DIR"), "/generated/server_images.rs"));
