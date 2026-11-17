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
            b.define("LINK_PLATFORM_WINDOWS", "1").define("NOMINMAX", None).define("_WIN32_WINNT", "0x0A00");
            if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
                // MinGW(MacからWindows版を作るとき)。libstdc++はDLLにせずexeに内蔵する(下で静的にリンク)
                b.flag("-std=c++17").flag_if_supported("-Wa,-mbig-obj").cpp_link_stdlib(None);
                // Link は <Windows.h> などを大文字で書いているが、MinGW のヘッダは小文字。
                // 大文字・小文字を区別するファイルシステム(LinuxからのクロスビルドやCI)でも見つかるよう、別名を用意する
                let dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("mingw-case");
                std::fs::create_dir_all(&dir).unwrap();
                for (alias, real) in [("Windows.h", "windows.h"), ("WinSock2.h", "winsock2.h"), ("WS2tcpip.h", "ws2tcpip.h")] {
                    std::fs::write(dir.join(alias), format!("#include <{real}>\n")).unwrap();
                }
                b.include(dir);
            } else {
                b.flag_if_supported("/std:c++17").flag_if_supported("/EHsc").flag_if_supported("/bigobj");
            }
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
            if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
                // MinGWのlibstdc++.aを静的にリンクする(別のPCでも libstdc++-6.dll 無しで動くように)
                let cxx = std::env::var("CXX_x86_64_pc_windows_gnu").unwrap_or_else(|_| "x86_64-w64-mingw32-g++".into());
                if let Ok(out) = std::process::Command::new(&cxx).arg("-print-file-name=libstdc++.a").output() {
                    let path = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
                    if let Some(dir) = path.parent() {
                        println!("cargo:rustc-link-search=native={}", dir.display());
                    }
                }
                println!("cargo:rustc-link-lib=static=stdc++");
            }
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
