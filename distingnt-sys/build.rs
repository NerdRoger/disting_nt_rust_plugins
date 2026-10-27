use std::{env, path::PathBuf, process::Command};

fn run(command: &mut Command) {
    let display = format!("{command:?}");
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("failed to run {display}: {error}"));
    assert!(status.success(), "command failed: {display}");
}

fn main() {
    println!("cargo:rerun-if-env-changed=ARM_NONE_EABI_CXX");
    println!("cargo:rerun-if-env-changed=ARM_NONE_EABI_AR");
    println!("cargo:rerun-if-env-changed=DISTINGNT_API_INCLUDE");
    println!("cargo:rerun-if-changed=shims");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let include_dir = env::var_os("DISTINGNT_API_INCLUDE")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("../distingNT_API/include"));
    assert!(
        include_dir.join("distingnt/api.h").is_file(),
        "Disting API header not found. Set DISTINGNT_API_INCLUDE to its include directory."
    );
    for header in [
        "api.h",
        "file.h",
        "wav.h",
        "microtuning.h",
        "slot.h",
        "serialisation.h",
    ] {
        println!(
            "cargo:rerun-if-changed={}",
            include_dir.join("distingnt").join(header).display()
        );
    }
    // Host-side tests use Rust stubs rather than the ARM-only native bridges.
    if env::var("TARGET").as_deref() != Ok("thumbv7em-none-eabihf") {
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let cxx = env::var("ARM_NONE_EABI_CXX").unwrap_or_else(|_| "arm-none-eabi-c++".into());
    let ar = env::var("ARM_NONE_EABI_AR").unwrap_or_else(|_| "arm-none-eabi-ar".into());
    let mut objects = Vec::new();
    let mut sources: Vec<_> = std::fs::read_dir(manifest_dir.join("shims"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|s| s == "cpp"))
        .collect();
    sources.sort();
    for source in sources {
        let object = out_dir
            .join(source.file_stem().unwrap())
            .with_extension("o");
        run(Command::new(&cxx)
            .args([
                "-std=c++23",
                "-mcpu=cortex-m7",
                "-mfpu=fpv5-d16",
                "-mfloat-abi=hard",
                "-mthumb",
                "-fno-exceptions",
                "-fno-rtti",
                "-Os",
                "-fPIC",
                "-ffunction-sections",
                "-Wall",
            ])
            .arg(format!("-I{}", include_dir.display()))
            .arg("-c")
            .arg(source)
            .arg("-o")
            .arg(&object));
        objects.push(object);
    }

    let archive = out_dir.join("libdistingnt_sys_shims.a");
    if archive.exists() {
        std::fs::remove_file(&archive).expect("remove stale shim archive");
    }
    run(Command::new(&ar).arg("rcs").arg(&archive).args(&objects));
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=distingnt_sys_shims");
    println!("cargo:metadata=shim_archive={}", archive.display());
}
