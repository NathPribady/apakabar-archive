// On Linux x86-64, ask for the system's standard program loader. A compiler from Nix (as on
// Replit) would otherwise write a loader path under /nix/store into the program, and the
// published app, which has no such path, would fail to start with "not found".
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    if target == "x86_64-unknown-linux-gnu" && std::path::Path::new("/lib64/ld-linux-x86-64.so.2").exists() {
        println!("cargo:rustc-link-arg-bins=-Wl,--dynamic-linker=/lib64/ld-linux-x86-64.so.2");
    }
}
