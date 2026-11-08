// Embeds the CutOut icon and version information into CutOut.exe on Windows.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../assets/icon/CutOut.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../assets/icon/CutOut.ico");
        res.set("ProductName", "CutOut");
        res.set("FileDescription", "CutOut — background removal and image editor");
        res.set("CompanyName", "CutOut");
        res.set("LegalCopyright", "Copyright © CutOut contributors");
        res.set("OriginalFilename", "CutOut.exe");
        res.set_manifest(include_str!("windows/CutOut.manifest"));
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed Windows resources: {e}");
        }
    }
}
