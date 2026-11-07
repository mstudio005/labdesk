// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

fn main() {
    let cfg = slint_build::CompilerConfiguration::new().with_style("fluent-light".into());
    slint_build::compile_with_config("ui/app.slint", cfg).expect("Slint compile failed");
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=assets");
    // Give labdesk.exe its icon on Windows. If the resource compiler is missing the app still builds.
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/labdesk.ico").set("ProductName", "LabDesk").set("FileDescription", "LabDesk");
        if let Err(e) = res.compile() { println!("cargo:warning=could not embed the app icon: {e}"); }
    }
}
