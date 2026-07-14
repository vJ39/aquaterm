# aquaterm 操作デモ撮影 (vhs)
# 生成: cd リポジトリ && vhs docs/aquaterm.tape
Output docs/demo.gif
Set Shell zsh
Set FontSize 12
Set Width 1600
Set Height 900
Set Padding 8
Set Framerate 12
Set TypingSpeed 40ms

Hide
Type "./target/release/aquaterm"
Enter
Sleep 3s
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

# ヘルプ(図鑑)
Type "?"
Sleep 4s
Type " "
Sleep 1s

# 終了
Type "q"
Sleep 800ms
