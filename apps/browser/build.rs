fn main() {
    // On Windows, compile the icon into the .exe (`assets/zipp.rc`). Without
    // the Windows SDK's resource compiler the build goes on, iconless.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/zipp.rc");
        println!("cargo:rerun-if-changed=assets/zipp.ico");
        if let Err(e) = embed_resource::compile("assets/zipp.rc", embed_resource::NONE).manifest_optional() {
            println!("cargo:warning=the app icon was not compiled in: {e}");
        }
        // A main thread with room to recurse over a deep page: 256 MiB of
        // stack, reserved and committed only as it is used. Growing it at run
        // time instead (stacker) made a fiber, whose stack Windows commits in
        // full -- 256 MiB of private memory from the start, nearly all of it
        // never touched.
        match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
            Ok("msvc") => println!("cargo:rustc-link-arg-bins=/STACK:268435456"),
            Ok("gnu") => println!("cargo:rustc-link-arg-bins=-Wl,--stack,268435456"),
            _ => {}
        }
    }
}
