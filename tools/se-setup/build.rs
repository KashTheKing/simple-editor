// The icon and the manifest: asInvoker (Windows otherwise elevates any exe named *setup* / *update* /
// *install*), Common Controls v6 (themed buttons, progress bar) and per-monitor DPI awareness.
fn main() {
    println!("cargo:rerun-if-changed=setup.manifest");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/app.ico");
        res.set_manifest_file("setup.manifest");
        res.set("ProductName", "Simple Editor");
        res.set("FileDescription", "Simple Editor setup");
        let _ = res.compile();
    }
}
