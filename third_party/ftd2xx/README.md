# FTDI D2XX

`mac/libftd2xx.dylib` は FTDI 公式の D2XX ライブラリ 1.4.36(macOS、x86_64 + arm64)です。
配布元: https://ftdichip.com/drivers/d2xx-drivers/ (D2XX1.4.36.dmg の release/build/libftd2xx.1.4.36.dylib)。
利用条件は FTDI のものに従います(`mac/ReadMe.rtf`)。

`scripts/build-mac.sh` が `DMX Controller.app/Contents/Frameworks/` に同梱し、アプリは最初にそこから読み込みます。
