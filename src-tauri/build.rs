fn main() {
    // `custom-protocol` (a default feature) embeds the built frontend. Give a clear message if it is missing.
    if std::env::var_os("CARGO_FEATURE_CUSTOM_PROTOCOL").is_some()
        && !std::path::Path::new("dist/index.html").exists()
    {
        panic!(
            "src-tauri/dist is missing. Run `npm ci --prefix ui && npm --prefix ui run build` first, \
             or build with --no-default-features to use the dev server."
        );
    }
    tauri_build::build()
}
