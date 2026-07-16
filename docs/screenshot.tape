# aquaterm スクリーンショット撮影 (vhs) - docs/screenshot.png の再撮影専用
# 生成: cd リポジトリ && cargo build --release && vhs docs/screenshot.tape
# HOMEを一時ディレクトリに向けて起動する(実際の~/.config/aquatermのセーブ
# データを読み書きしないようにするため)。
# aquaterm.tapeと解像度・フォント設定を揃え、demo.gifと見た目の一貫性を保つ。
Output "/tmp/aquaterm-screenshot-throwaway.gif"
Set Shell zsh
Set FontSize 12
Set Width 1600
Set Height 900
Set Padding 8
Set Framerate 12
Set TypingSpeed 40ms

Hide
Type "HOME=/tmp/aquaterm-screenshot-home ./target/release/aquaterm"
Enter
Sleep 3s
# 初回起動(セーブなし)は自動でヘルプ画面が開くため、表示前に閉じておく
Type " "
Sleep 300ms
Show
Sleep 1s

# 賑やかな見た目にするため、餌やりとピラニア・タコ・クジラを追加してから撮影
Type "f"
Sleep 1s
Type "S"
Sleep 300ms
Type "O"
Sleep 300ms
Type "W"
Sleep 2s

Screenshot docs/screenshot.png
Sleep 300ms

Hide
Type "q"
Sleep 500ms
