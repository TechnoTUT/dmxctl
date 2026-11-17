//! 開発・確認用: 画面を画像に保存する。環境変数 DMXCTL_SHOT=保存先.png で起動すると、
//! 少し待ってから自分のウィンドウを撮ってPNGで保存し、アプリを閉じる(OSの画面収録の許可は要らない)。
//! DMXCTL_TAB=タブ名 で、撮るタブを選べる

use eframe::egui;
use std::path::{Path, PathBuf};

pub struct Shot {
    path: PathBuf,
    frames: u32,
    wait: u32,
    requested: bool,
}

/// 撮影を頼むまでに待つフレーム数(フォントの読み込みや初回の配置が落ち着くまで)。
/// DMXCTL_SHOT_FRAMES=フレーム数 で延ばせる(撮る前に、操作を入れたいとき)
const WAIT_FRAMES: u32 = 20;

impl Shot {
    pub fn from_env() -> Option<Shot> {
        let path = std::env::var_os("DMXCTL_SHOT").filter(|v| !v.is_empty())?;
        let wait = std::env::var("DMXCTL_SHOT_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(WAIT_FRAMES);
        Some(Shot { path: PathBuf::from(path), frames: 0, wait, requested: false })
    }

    /// 毎フレーム呼ぶ。撮れたら保存してウィンドウを閉じる
    pub fn tick(&mut self, ctx: &egui::Context) {
        self.frames += 1;
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            if let Err(e) = save_png(&self.path, &image) {
                eprintln!("画像を保存できませんでした: {e}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.requested && self.frames >= self.wait {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.requested = true;
        }
        ctx.request_repaint();
    }
}

/// RGBAのPNG(無圧縮のzlib)で保存する
pub fn save_png(path: &Path, img: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = img.size;
    let mut raw = Vec::with_capacity((w * 4 + 1) * h);
    for row in img.pixels.chunks(w) {
        raw.push(0); // フィルタなし
        for p in row {
            raw.extend_from_slice(&p.to_srgba_unmultiplied());
        }
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(0xFFFF).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push((i + 1 == blocks.len()) as u8);
        let n = b.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8bit RGBA
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png)
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_match_known_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
