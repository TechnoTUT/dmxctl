//! モデル層。保存する設定 `project.json` の形、機材定義、色・動き・ピクセル・キーの表。
//! 機器にも画面にも出力ループにも依存しない、データとその規則だけの部分。
//!
//! 出力ループと機器とのやりとりは `dmxctl-core` に、画面は `dmxctl-ui` にある。
pub mod apcmini;
pub mod color;
pub mod fixture;
pub mod keys;
pub mod layout;
pub mod motion;
pub mod pixel;
pub mod project;
