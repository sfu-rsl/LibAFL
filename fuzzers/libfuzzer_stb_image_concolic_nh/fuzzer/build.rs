// build.rs

use std::{
    env,
    io::{stdout, Write},
    path::{Path, PathBuf},
    process::exit,
};

fn main() {
    if !cfg!(target_os = "linux") {
        println!("cargo:warning=Only linux host is supported for now.");
        exit(0);
    }

    let out_path = PathBuf::from(&env::var_os("OUT_DIR").unwrap());

    let symcc_dir = clone_and_build_symcc(&out_path);

    let runtime_dir = std::env::current_dir().unwrap().join("..").join("runtime");

    // Build the runtime
    std::process::Command::new("cargo")
        .current_dir(&runtime_dir)
        .env_remove("CARGO_TARGET_DIR")
        .arg("build")
        .arg("--release")
        .status()
        .expect("Failed to build runtime");

    std::fs::copy(
        runtime_dir
            .join("target")
            .join("release")
            .join("libSymRuntime.so"),
        runtime_dir.join("libSymRuntime.so"),
    )
    .unwrap();

    if !runtime_dir.join("libSymRuntime.so").exists() {
        println!("cargo:warning=Runtime not found. Build it first.");
        exit(1);
    }

    // SymCC.
    std::env::set_var("CC", symcc_dir.join("symcc"));
    println!(
        "The symcc path {}",
        symcc_dir.join("symcc").to_string_lossy()
    );
    std::env::set_var("CXX", symcc_dir.join("sym++"));
    std::env::set_var("SYMCC_RUNTIME_DIR", runtime_dir);

    println!("cargo:rerun-if-changed=harness_symcc.c");

    let output = cc::Build::new()
        .flag("-Wno-sign-compare")
        .flag("-Wunused-but-set-variable")
        .flag("-fsanitize-coverage=trace-pc-guard,trace-cmp")
        .cargo_metadata(false)
        .get_compiler()
        .to_command()
        .arg("./harness_symcc.c")
        .args(["-o", "target_symcc.out"])
        .arg("-lm")
        .output()
        .expect("failed to execute symcc");
    if !output.status.success() {
        println!("cargo:warning=Building the target with SymCC failed");
        let mut stdout = stdout();
        stdout
            .write_all(&output.stderr)
            .expect("failed to write cc error message to stdout");
        exit(1);
    }

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=harness_symcc.c");
}

fn clone_and_build_symcc(out_path: &Path) -> PathBuf {
    let repo_dir = out_path.join("libafl_symcc_src");
    if !repo_dir.exists() {
        symcc_libafl::clone_symcc(&repo_dir);
    }

    symcc_libafl::build_symcc(&repo_dir)
}
