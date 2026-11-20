// The icon and the manifest: asInvoker (Windows otherwise elevates any exe named *setup* / *update* /
// *install*), Common Controls v6 (themed buttons, progress bar) and per-monitor DPI awareness.
// Runs for a Windows target on any host (MSVC's rc.exe, or mingw's windres when cross-compiling).
fn main() {
    println!("cargo:rerun-if-changed=setup.manifest");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app.ico");
    res.set_manifest_file("setup.manifest");
    res.set("ProductName", "Simple Editor");
    res.set("FileDescription", "Simple Editor setup");
    if let Err(e) = res.compile() {
        // without the manifest Windows would ask for admin rights: make that loud
        println!("cargo:warning=no icon/manifest compiled in: {e}");
    }
}
