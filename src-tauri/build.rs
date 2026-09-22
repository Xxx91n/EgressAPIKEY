//! Tauri build hook: generates the context for `tauri::generate_context!`.
//! Required by every Tauri 2 application.

fn main() {
    tauri_build::build();
    embed_test_manifest();
}

// tauri-specta pilot: the bindings export integration test
// links the shell lib, whose tauri dependency statically imports comctl32
// v6 entrypoints (SetWindowSubclass / TaskDialogIndirect via muda). The
// main bin gets its SxS manifest from tauri_build, but a TEST binary does
// not - so it binds comctl32 v5 at load time and dies with
// STATUS_ENTRYPOINT_NOT_FOUND (0xC0000139) before any test runs. Embed
// the manifest into integration test targets as well (MSVC only).
fn embed_test_manifest() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let manifest = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
        "<assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\" manifestVersion=\"1.0\">\n",
        "  <dependency>\n",
        "    <dependentAssembly>\n",
        "      <assemblyIdentity type=\"win32\" name=\"Microsoft.Windows.Common-Controls\" version=\"6.0.0.0\" processorArchitecture=\"*\" publicKeyToken=\"6595b64144ccf1df\" language=\"*\"/>\n",
        "    </dependentAssembly>\n",
        "  </dependency>\n",
        "</assembly>\n",
    );
    let path = std::path::Path::new(&out_dir).join("test-manifest.manifest");
    std::fs::write(&path, manifest).expect("write test-manifest.manifest");
    println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
        path.display()
    );
}
