fn main() {
    println!("cargo:rerun-if-changed=src/platform/macos_bt_bridge.m");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/platform/macos_bt_bridge.m")
            .flag("-fobjc-arc")
            .flag("-ObjC")
            .compile("murmur_macos_bt");

        println!("cargo:rustc-link-lib=framework=IOBluetooth");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=AppKit");
    }
}
