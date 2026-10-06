// Ableton Link(C++、ヘッダのみ)を小さな窓口(native/link_shim.cpp)ごと静的にリンクする
fn main() {
    println!("cargo:rerun-if-changed=native/link_shim.cpp");
    println!("cargo:rerun-if-changed=build.rs");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    println!("cargo:rerun-if-env-changed=DMXCTL_CHECK_ONLY");
    // 他のOS向けに `cargo check` で確かめるときだけ使う(C++はビルドしないので、実行ファイルは作れない)
    if std::env::var_os("DMXCTL_CHECK_ONLY").is_some() {
        return;
    }
    let mut b = cc::Build::new();
    b.cpp(true)
        .file("native/link_shim.cpp")
        .include("third_party/link/include")
        .include("third_party/link/asio/include")
        .warnings(false);
    match os.as_str() {
        "windows" => {
            b.define("LINK_PLATFORM_WINDOWS", "1").define("NOMINMAX", None).define("_WIN32_WINNT", "0x0A00").flag_if_supported("/std:c++17").flag_if_supported("/EHsc").flag_if_supported("/bigobj");
        }
        "macos" => {
            b.define("LINK_PLATFORM_MACOSX", "1").flag("-std=c++17");
        }
        _ => {
            b.define("LINK_PLATFORM_LINUX", "1").flag("-std=c++17");
        }
    }
    b.compile("dmx_link");
    match os.as_str() {
        "windows" => {
            println!("cargo:rustc-link-lib=iphlpapi");
            println!("cargo:rustc-link-lib=ws2_32");
        }
        "macos" => println!("cargo:rustc-link-lib=c++"),
        _ => {
            println!("cargo:rustc-link-lib=stdc++");
            println!("cargo:rustc-link-lib=atomic");
        }
    }
}
