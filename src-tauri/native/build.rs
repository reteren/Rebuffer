fn main() {
    slint_build::compile("ui/app.slint").expect("ui/app.slint compiles");

    // The About panel shows the version the installer carries, read from the
    // same tauri.conf.json the WebView build's About panel reads.
    let conf = std::path::Path::new("../tauri.conf.json");
    println!("cargo:rerun-if-changed=../tauri.conf.json");
    let version = std::fs::read_to_string(conf)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("version")?.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=REBUFFER_VERSION={version}");
}
