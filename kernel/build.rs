//! Kernel build script (Phase 2 — ELF loader migration).
//!
//! Responsibilities:
//!   1. Build each user-space server image (`vga`, `keyboard`, `init`, `shell`)
//!      as an ET_EXEC ELF at load base `0x10000000` via `userspace/build.sh`.
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

/// Reject stale/mislinked ELFs at build time, before the kernel embeds an image
/// whose PT_LOAD segments overlap the supervisor identity map or VGA alias.
fn validate_user_elf(path: &std::path::Path, image_name: &str) {
    const USER_BASE: u64 = 0x1000_0000;
    const USER_END: u64 = 0x2000_0000;
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        panic!("failed to read user ELF `{image_name}` ({}): {e}", path.display())
    });
    let u16_at = |off: usize| -> u16 {
        u16::from_le_bytes(bytes[off..off + 2].try_into().expect("checked ELF field"))
    };
    let u32_at = |off: usize| -> u32 {
        u32::from_le_bytes(bytes[off..off + 4].try_into().expect("checked ELF field"))
    };
    let u64_at = |off: usize| -> u64 {
        u64::from_le_bytes(bytes[off..off + 8].try_into().expect("checked ELF field"))
    };

    assert!(bytes.len() >= 64, "user ELF `{image_name}` is truncated");
    assert_eq!(&bytes[..4], b"\x7fELF", "user ELF `{image_name}` has bad magic");
    assert_eq!(bytes[4], 2, "user ELF `{image_name}` is not ELF64");
    assert_eq!(bytes[5], 1, "user ELF `{image_name}` is not little-endian");
    assert_eq!(u16_at(16), 2, "user ELF `{image_name}` is not ET_EXEC");
    assert_eq!(u16_at(18), 0x3E, "user ELF `{image_name}` is not x86-64");

    let entry = u64_at(24);
    let phoff = usize::try_from(u64_at(32)).expect("ELF program-header offset fits usize");
    let phentsize = u16_at(54) as usize;
    let phnum = u16_at(56) as usize;
    assert!(phentsize >= 56 && phnum > 0, "user ELF `{image_name}` has no program headers");
    let phend = phoff
        .checked_add(phentsize.checked_mul(phnum).expect("program-header table size overflow"))
        .expect("program-header table end overflow");
    assert!(phend <= bytes.len(), "user ELF `{image_name}` program headers are truncated");

    let mut has_load = false;
    let mut entry_is_executable = false;
    for i in 0..phnum {
        let off = phoff + i * phentsize;
        if u32_at(off) != 1 { // PT_LOAD
            continue;
        }
        has_load = true;
        let flags = u32_at(off + 4);
        let vaddr = u64_at(off + 16);
        let filesz = u64_at(off + 32);
        let memsz = u64_at(off + 40);
        let end = vaddr.checked_add(memsz).expect("PT_LOAD virtual range overflow");
        assert!(filesz <= memsz, "user ELF `{image_name}` has filesz > memsz");
        assert!(vaddr >= USER_BASE && end <= USER_END, "user ELF `{image_name}` PT_LOAD [{vaddr:#x}, {end:#x}) is outside its user window");
        if flags & 1 != 0 && entry >= vaddr && entry < end {
            entry_is_executable = true;
        }
    }
    assert!(has_load, "user ELF `{image_name}` contains no PT_LOAD segments");
    assert!(entry_is_executable, "user ELF `{image_name}` entry {entry:#x} is not in an executable PT_LOAD");
}

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
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("../userspace/src").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("../userspace/.cargo/config.toml").display()
    );
    // Rebuild when the interrupt server's own tooling changes.
    println!("cargo:rerun-if-changed={}", interrupt_build_sh.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("../interrupt_server/Cargo.toml").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("../interrupt_server/src").display()
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
        validate_user_elf(&elf, name);

        let upper = name.to_uppercase();
        generated.push_str(&format!(
            "/// `{name}` user-space server image (ET_EXEC @ 0x10000000).\n\
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
    validate_user_elf(&isr_elf, "interrupt_server");

    generated.push_str(
        "/// User-space interrupt server image (ET_EXEC @ 0x10000000).\n\
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
