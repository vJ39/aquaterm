# aquaterm 操作デモ撮影 (vhs)
# 生成: cd リポジトリ && cargo build --release && vhs docs/aquaterm.tape
# HOMEを一時ディレクトリに向けて起動する(実際の~/.config/aquatermのセーブ
# データを読み書きしないようにするため。爆発デモで水槽内が全滅する操作を
# 含むので、実データを巻き込まない工夫が必須)。
Output docs/demo.gif
Set Shell zsh
Set FontSize 12
Set Width 1600
Set Height 900
Set Padding 8
Set Framerate 12
Set TypingSpeed 40ms

Hide
Type "HOME=/tmp/aquaterm-demo-home ./target/release/aquaterm"
Enter
Sleep 3s
# 初回起動(セーブなし)は自動でヘルプ画面が開くため、表示前に閉じておく
Type " "
Sleep 300ms
Show
Sleep 2s

# カーソルを動かして水槽を見て回る
Right@120ms 8
Down@120ms 4
Sleep 1s

# 餌やり
Type "f"
Sleep 3s

# ガラスを叩く(驚いて逃げる)
Type "t"
Sleep 2s

# トントン(寄ってくる)
Type "T"
Sleep 2s

# 薬
Type "m"
Sleep 2s

# 複数水槽の選択画面を一瞬開いて閉じる
Type "N"
Sleep 1.2s
Escape
Sleep 600ms

# 捕食シーン: 速度を上げてピラニアを投入し、他の魚を狩る様子を見せる
Type "]"
Sleep 300ms
Type "]"
Sleep 300ms
Type "]"
Sleep 500ms
Type "S"
Sleep 300ms
Type "S"
Sleep 10s
Type "["
Sleep 300ms
Type "["
Sleep 300ms
Type "["
Sleep 500ms

# クジラを投入し、デバッグ操作(Bキー)で60秒待たずに即大爆発させる
# (赤いフラッシュ・肉片エフェクト)
Type "W"
Sleep 1.5s
Type "B"
Sleep 3s

# ヘルプ(図鑑)
Type "?"
Sleep 4s
Type " "
Sleep 1s

# 終了
Type "q"
Sleep 800ms
