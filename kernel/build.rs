//! Kernel build script (Phase 2 — ELF loader migration).
//!
//! Responsibilities:
//!   1. Build each user-space server image (`vga`, `keyboard`, `init`, `shell`)
//!      as an ET_EXEC ELF at load base `0x400000` via `userspace/build.sh`.
//!   2. Build the user-space interrupt server (the IRQ router) via
//!      `interrupt_server/build.sh`.
//!   3. Copy the resulting ELFs into `$OUT_DIR/server_elfs/<name>.elf`.
//!   4. Generate a Rust module that embeds those bytes with `include_bytes!`,
//!      exposed as `crate::server_images::<NAME>_ELF` and
//!      `crate::server_images::INTERRUPT_SERVER_ELF`.
//!
//! The loader reads these byte slices at boot and feeds them to
//! `elf::load_elf` instead of jumping to inline function addresses.

use std::path::PathBuf;
use std::process::Command;

/// User-space server images the kernel must embed and load at boot.
const SERVERS: &[&str] = &["vga", "keyboard", "init", "shell", "fs_server"];

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let build_sh = manifest_dir.join("../userspace/build.sh");
    let interrupt_build_sh = manifest_dir.join("../interrupt_server/build.sh");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set at build time"));
    let elf_dir = out_dir.join("server_elfs");
    let gen_dir = out_dir.join("generated");

    std::fs::create_dir_all(&elf_dir).expect("failed to create server_elfs dir");
    std::fs::create_dir_all(&gen_dir).expect("failed to create generated dir");

    // Rebuild when the userspace build tooling changes.
    println!("cargo:rerun-if-changed={}", build_sh.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("../userspace/Cargo.toml").display()
    );
    // Rebuild when the interrupt server's own tooling changes.
    println!("cargo:rerun-if-changed={}", interrupt_build_sh.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("../interrupt_server/Cargo.toml").display()
    );

    let mut generated = String::new();
    for name in SERVERS {
        let status = Command::new("/bin/bash")
            .arg(build_sh.display().to_string())
            .arg(name)
            .arg("--out")
            .arg(&elf_dir)
            .status()
            .expect("failed to invoke userspace/build.sh");

        if !status.success() {
            panic!(
                "userspace build failed for `{}` (exit {:?})",
                name,
                status.code()
            );
        }

        let elf = elf_dir.join(format!("{name}.elf"));
        if !elf.exists() {
            panic!("expected ELF not produced for `{}`: {}", name, elf.display());
        }

        let upper = name.to_uppercase();
        generated.push_str(&format!(
            "/// `{name}` user-space server image (ET_EXEC @ 0x400000).\n\
             pub const {upper}_ELF: &[u8] =\n\
             include_bytes!(concat!(env!(\"OUT_DIR\"), \"/server_elfs/{name}.elf\"));\n\n",
        ));
    }

    // Build the user-space interrupt server (IRQ router). Unlike the per-server
    // images it takes no `<name>` argument; `interrupt_server/build.sh` builds a
    // single fixed binary and emits `interrupt_server.elf`.
    let isr_status = Command::new("/bin/bash")
        .arg(interrupt_build_sh.display().to_string())
        .arg("--out")
        .arg(&elf_dir)
        .status()
        .expect("failed to invoke interrupt_server/build.sh");

    if !isr_status.success() {
        panic!("interrupt_server build failed (exit {:?})", isr_status.code());
    }

    let isr_elf = elf_dir.join("interrupt_server.elf");
    if !isr_elf.exists() {
        panic!(
            "expected interrupt_server ELF not produced: {}",
            isr_elf.display()
        );
    }

    generated.push_str(
        "/// User-space interrupt server image (ET_EXEC @ 0x400000).\n\
         ///\n\
         /// Loaded at boot when `INTERRUPT_SERVER_PID != 0`; the kernel routes\n\
         /// hardware IRQs to it instead of delivering them directly.\n\
         pub const INTERRUPT_SERVER_ELF: &[u8] =\n\
         include_bytes!(concat!(env!(\"OUT_DIR\"), \"/server_elfs/interrupt_server.elf\"));\n\n",
    );

    let gen_file = gen_dir.join("server_images.rs");
    std::fs::write(&gen_file, generated).expect("failed to write generated server_images.rs");
    println!("cargo:rerun-if-changed={}", gen_file.display());
}
