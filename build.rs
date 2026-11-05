// Bettet Icon und Versionsangaben in die Windows-Exe ein. Fehlt das Windows-SDK (rc.exe),
// wird die Exe trotzdem gebaut, nur eben ohne Icon. Unter Linux gibt es hier nichts zu tun.
fn main() {
    println!("cargo:rerun-if-changed=assets/storage-map.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if !std::path::Path::new("assets/storage-map.ico").exists() {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/storage-map.ico");
    res.set("ProductName", "Storage Map");
    res.set("FileDescription", "Storage Map");
    res.set("LegalCopyright", "Storage Map contributors, MIT License");
    if let Err(e) = res.compile() {
        println!("cargo:warning=Icon not embedded: {e}");
    }
}
