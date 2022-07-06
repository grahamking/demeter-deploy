// Build script to compile the assembler part
// It then gets embedded in the binary.

use std::env::var;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    // only run if asm source changes
    println!("cargo:rerun-if-changed=asm/main.s");

    let mut asm_dir = PathBuf::from(var("CARGO_MANIFEST_DIR").unwrap());
    asm_dir.push("asm");

    // assemble
    let out = Command::new("nasm")
        .args(["-felf64", "-o", "main.o", "main.s"])
        .current_dir(&asm_dir)
        .output()
        .expect("Failed running nasm to assemble helper");
    if !out.status.success() {
        println!("nasm stdout: {}", String::from_utf8_lossy(&out.stdout));
        println!("nasm stderr: {}", String::from_utf8_lossy(&out.stderr));
        return ExitCode::from(1);
    }

    // link
    // -s strips symbols, link `strip` cmd
    // -n and -N prevent page aligning which bloats the binary
    let out = Command::new("ld")
        .args(["-s", "-n", "-N", "-o", "rcple-h", "main.o"])
        .current_dir(asm_dir)
        .output()
        .expect("Failed running ld to link helper");
    if !out.status.success() {
        println!("ld stdout: {}", String::from_utf8_lossy(&out.stdout));
        println!("ld stderr: {}", String::from_utf8_lossy(&out.stderr));
        return ExitCode::from(2);
    }

    ExitCode::from(0)
}
