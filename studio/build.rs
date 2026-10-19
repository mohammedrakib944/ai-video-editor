//! Embeds the app icon into the Windows executable (Explorer, taskbar, shortcuts).
//! The icon is drawn by `src/icon.rs`, the same code the window icon uses.

#[path = "src/icon.rs"]
#[allow(dead_code)]
mod icon;

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let ico = out.join("app.ico");
    let rc = out.join("app.rc");
    if let Err(e) = std::fs::write(&ico, icon::ico()).and_then(|_| std::fs::write(&rc, format!("1 ICON \"{}\"\n", ico.display().to_string().replace('\\', "/")))) {
        println!("cargo:warning=could not write the app icon: {e}");
        return;
    }
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let linked = if msvc {
        let res = out.join("app.res");
        let ok = Command::new("rc.exe").arg("/nologo").arg("/fo").arg(&res).arg(&rc).status().is_ok_and(|s| s.success());
        ok.then_some(res)
    } else {
        // MinGW's windres: plain on Windows, prefixed when cross-compiling from Linux.
        let obj = out.join("app_icon.o");
        let tools = [std::env::var("WINDRES").ok(), Some("windres".into()), Some("x86_64-w64-mingw32-windres".into())];
        let ok = tools.into_iter().flatten().any(|tool| Command::new(tool).arg("--input").arg(&rc).arg("--output").arg(&obj).arg("--output-format=coff").status().is_ok_and(|s| s.success()));
        ok.then_some(obj)
    };
    match linked {
        Some(file) => println!("cargo:rustc-link-arg-bins={}", file.display()),
        None => println!("cargo:warning=no resource compiler found (windres or rc.exe): the exe gets no embedded icon"),
    }
}
