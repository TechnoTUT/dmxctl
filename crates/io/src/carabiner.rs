//! Carabiner のダウンロードと起動。Ableton Link に参加する別のプログラムで、src/link.rs がつなぐ。
//!
//! Carabiner は GPL なので、このアプリには同梱しない。使う人が自分でボタンやコマンドを押したときだけ、
//! 配布元の GitHub Releases から取ってくる。ファイルは、コードに固定したチェックサムと合ったときだけ使う。

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

pub const VERSION: &str = "1.2.0";

/// OSごとに使う形が違うので、使わない形は警告させない
#[allow(dead_code)]
#[derive(Clone, Copy)]
enum Kind {
    Gz,
    Zip,
    Dmg,
}

struct Asset {
    file: &'static str,
    sha256: &'static str,
    kind: Kind,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const ASSET: Option<Asset> = Some(Asset { file: "Carabiner_Linux_x64.gz", sha256: "f9b3a8384cd20d78ec28cac6a23ba655428163c3fc872f2af9299101cd28dc13", kind: Kind::Gz });
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const ASSET: Option<Asset> = Some(Asset { file: "Carabiner_Linux_arm.gz", sha256: "cb636ecaa1ac2c3b797cba58688e7b68e9d75d0dd02222f9b06ddea18a0a78ed", kind: Kind::Gz });
#[cfg(target_os = "macos")]
const ASSET: Option<Asset> = Some(Asset { file: "Carabiner_Mac.dmg", sha256: "13a53b16fb044fbfde46122804948e6d95137cc0a7812c3e7b34767817b5bdae", kind: Kind::Dmg });
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const ASSET: Option<Asset> = Some(Asset { file: "Carabiner_Win_x64.zip", sha256: "d0acd5f739cbed3cff9dddd5e816761d4fee5c9e8716a5f948fa4ce396c6157a", kind: Kind::Zip });
#[cfg(not(any(
    all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")),
    target_os = "macos",
    all(target_os = "windows", target_arch = "x86_64")
)))]
const ASSET: Option<Asset> = None;

/// このPCの向けの Carabiner が配られているか
pub fn supported() -> bool {
    ASSET.is_some()
}

pub fn url() -> Option<String> {
    ASSET.as_ref().map(|a| format!("https://github.com/Deep-Symmetry/carabiner/releases/download/v{VERSION}/{}", a.file))
}

fn dir() -> PathBuf {
    dmxctl_model::project::config_dir().join("carabiner")
}

/// 入れた Carabiner の場所
pub fn exe_path() -> PathBuf {
    dir().join(if cfg!(windows) { "Carabiner.exe" } else { "Carabiner" })
}

pub fn installed() -> bool {
    exe_path().is_file()
}

#[derive(Clone, Debug, PartialEq)]
pub enum InstallState {
    Idle,
    Running,
    Done,
    Failed(String),
}

static STATE: Mutex<InstallState> = Mutex::new(InstallState::Idle);

pub fn install_state() -> InstallState {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set_state(s: InstallState) {
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

/// 裏のスレッドで入れる。すでに入れている最中なら何もしない
pub fn install_in_background() {
    {
        let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if *g == InstallState::Running {
            return;
        }
        *g = InstallState::Running;
    }
    std::thread::spawn(|| match install() {
        Ok(_) => set_state(InstallState::Done),
        Err(e) => set_state(InstallState::Failed(format!("{e:#}"))),
    });
}

/// 配布元から取って、チェックサムを確かめ、設定フォルダに置く。置いた場所を返す
pub fn install() -> anyhow::Result<PathBuf> {
    use anyhow::{Context, bail};
    let asset = ASSET.as_ref().context("Carabiner is not distributed for this OS or CPU")?;
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    let part = dir.join("download.part");
    let _cleanup = Cleanup(part.clone());
    let mut cmd = Command::new("curl");
    cmd.args(["--fail", "--location", "--silent", "--show-error", "--max-time", "300", "--output"]).arg(&part).arg(url().unwrap());
    let out = hide_window(&mut cmd).output().context("could not run curl")?;
    if !out.status.success() {
        bail!("download failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let data = std::fs::read(&part)?;
    let got = sha256_hex(&data);
    if got != asset.sha256 {
        bail!("checksum mismatch for {}: expected {}, got {got}", asset.file, asset.sha256);
    }
    let exe = exe_path();
    let tmp = dir.join("Carabiner.part");
    let _cleanup2 = Cleanup(tmp.clone());
    extract(asset.kind, &part, &data, &tmp, &dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&tmp, &exe)?;
    Ok(exe)
}

/// 途中で失敗しても、書きかけのファイルを残さない
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

#[allow(unused_variables)]
fn extract(kind: Kind, archive: &std::path::Path, data: &[u8], out: &std::path::Path, dir: &std::path::Path) -> anyhow::Result<()> {
    match kind {
        #[cfg(unix)]
        Kind::Gz => {
            let mut r = flate2::read::GzDecoder::new(data);
            let mut f = std::fs::File::create(out)?;
            std::io::copy(&mut r, &mut f)?;
            std::io::Write::flush(&mut f)?;
            Ok(())
        }
        #[cfg(windows)]
        Kind::Zip => {
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(data))?;
            let mut f = z.by_name("Carabiner.exe")?;
            let mut o = std::fs::File::create(out)?;
            std::io::copy(&mut f, &mut o)?;
            Ok(())
        }
        #[cfg(target_os = "macos")]
        Kind::Dmg => {
            use anyhow::bail;
            let mnt = dir.join("mnt");
            std::fs::create_dir_all(&mnt)?;
            let attach = Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-noverify", "-mountpoint"]).arg(&mnt).arg(archive).output()?;
            if !attach.status.success() {
                bail!("hdiutil attach failed: {}", String::from_utf8_lossy(&attach.stderr).trim());
            }
            let result = (|| -> anyhow::Result<()> {
                let src = find_file(&mnt, "Carabiner", 3).ok_or_else(|| anyhow::anyhow!("Carabiner was not found in the disk image"))?;
                std::fs::copy(src, out)?;
                Ok(())
            })();
            let _ = Command::new("hdiutil").args(["detach", "-force"]).arg(&mnt).output();
            let _ = std::fs::remove_dir(&mnt);
            result
        }
        #[allow(unreachable_patterns)]
        _ => anyhow::bail!("unsupported archive for this OS"),
    }
}

#[cfg(target_os = "macos")]
fn find_file(dir: &std::path::Path, name: &str, depth: u32) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_file() && p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
        if depth > 0 && p.is_dir() {
            if let Some(f) = find_file(&p, name, depth - 1) {
                return Some(f);
            }
        }
    }
    None
}

#[cfg(windows)]
fn hide_window(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000)
}

#[cfg(not(windows))]
fn hide_window(cmd: &mut Command) -> &mut Command {
    cmd
}

/// 入れてある Carabiner を、指定のポートで起動する。入れていなければ None
pub fn spawn(port: u16) -> Option<Child> {
    if !installed() {
        return None;
    }
    let mut cmd = Command::new(exe_path());
    cmd.args(["--daemon", "--port"]).arg(port.to_string()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    hide_window(&mut cmd).spawn().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_value() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn url_points_to_the_pinned_release() {
        if let Some(u) = url() {
            assert!(u.starts_with("https://github.com/Deep-Symmetry/carabiner/releases/download/v1.2.0/Carabiner_"), "{u}");
        }
    }
}
