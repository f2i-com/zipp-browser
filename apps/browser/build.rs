fn main() {
    // On Windows, compile the icon into the .exe (`assets/zipp.rc`). Without
    // the Windows SDK's resource compiler the build goes on, iconless.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/zipp.rc");
        println!("cargo:rerun-if-changed=assets/zipp.ico");
        if let Err(e) = embed_resource::compile("assets/zipp.rc", embed_resource::NONE).manifest_optional() {
            println!("cargo:warning=the app icon was not compiled in: {e}");
        }
    }
}
