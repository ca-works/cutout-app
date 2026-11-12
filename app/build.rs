// Embeds the CutOut icon and version information into CutOut.exe on Windows.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../assets/icon/CutOut.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../assets/icon/CutOut.ico");
        res.set("ProductName", "CutOut");
        res.set("ProductVersion", env!("CARGO_PKG_VERSION"));
        res.set("FileVersion", env!("CARGO_PKG_VERSION"));
        let version = [
            env!("CARGO_PKG_VERSION_MAJOR"),
            env!("CARGO_PKG_VERSION_MINOR"),
            env!("CARGO_PKG_VERSION_PATCH"),
            env!("CARGO_PKG_VERSION_PRE").rsplit_once('.').map_or("0", |(_, revision)| revision),
        ].into_iter().fold(0u64, |number, part| (number << 16) | u64::from(part.parse::<u16>().unwrap()));
        res.set_version_info(winresource::VersionInfo::FILEVERSION, version);
        res.set_version_info(winresource::VersionInfo::PRODUCTVERSION, version);
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
