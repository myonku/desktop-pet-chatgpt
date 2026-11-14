fn main() {
    for file in ["icons.rc", "app.icon.ico", "tray.icon.ico"] {
        println!("cargo:rerun-if-changed=assets/icon/{file}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        compile_icons();
    }
}

#[cfg(windows)]
fn compile_icons() {
    use std::{env, path::PathBuf};

    let target = env::var("TARGET").unwrap();
    assert_eq!(env::var("CARGO_CFG_TARGET_ENV").as_deref(), Ok("msvc"));
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("icons.res");
    let mut rc = find_msvc_tools::find(&target, "rc.exe")
        .unwrap_or_else(|| std::process::Command::new("rc.exe"));
    let status = rc
        .current_dir("assets/icon")
        .arg("/nologo")
        .arg("/fo")
        .arg(&out)
        .arg("icons.rc")
        .status()
        .expect("Windows SDK resource compiler (rc.exe) is required");
    assert!(
        status.success(),
        "could not compile the embedded Windows icons"
    );
    println!(
        "cargo:rustc-link-arg-bin=desktop-pet-chatgpt={}",
        out.display()
    );
}

#[cfg(not(windows))]
fn compile_icons() {
    panic!("Building Windows icons requires the Windows MSVC toolchain and SDK");
}
