fn main() {
    // tauri-build does not re-run on an icon or config change by itself: a
    // regenerated icon silently kept its stale cached RGBA (a 128x128 icon kept
    // an 8192-byte cache until build.rs was touched). Watch the inputs that
    // tauri-codegen embeds so a change always rebuilds the cache.
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=icons");
    println!("cargo:rerun-if-changed=capabilities");
    println!("cargo:rerun-if-changed=frontend");
    tauri_build::build()
}
