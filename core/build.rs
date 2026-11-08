// ONNX Runtime's CoreML execution provider uses `@available`, which needs
// `___isPlatformVersionAtLeast` from clang's compiler-rt. Rust links with
// -nodefaultlibs, so add the runtime explicitly on macOS.
fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("apple-darwin") && std::env::var("CARGO_FEATURE_AI").is_ok() {
        if let Ok(out) = std::process::Command::new("clang").arg("--print-runtime-dir").output() {
            let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !dir.is_empty() {
                println!("cargo:rustc-link-search=native={dir}");
                println!("cargo:rustc-link-lib=static=clang_rt.osx");
            }
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
