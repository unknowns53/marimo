# marimo の開発

marimo をソースからビルドする人、`install` が何を書き換えるかを確かめたい人、コードを直したい人のための文書です。marimo の概要と導入の手順は [README](../README.md) にあります。

- [ソースからビルドする](#ソースからビルドする)
- [フックと statusLine の登録](#フックと-statusline-の登録)
- [ファイルとデータ](#ファイルとデータ)
- [プライバシーとセキュリティ](#プライバシーとセキュリティ)
- [リポジトリの構成](#リポジトリの構成)
- [テストと検査](#テストと検査)
- [アイコンを描き直す](#アイコンを描き直す)
- [仕組みの概要](#仕組みの概要)

## ソースからビルドする

この節では、ビルドに必要なもの、できあがるもの、macOS での署名について説明します。ビルドのコマンドは、README の [1. ビルドする](../README.md#1-ビルドする)にあります。

### 必要なもの

| ツール | バージョン | 使う場面 |
| --- | --- | --- |
| Rust（rustc と cargo） | 1.90 以降（`Cargo.toml` の `rust-version`）、edition 2024 | フックのコマンド `marimo-hook` とアプリ本体のビルド |
| Node.js と npm | Node.js 22.12 以上の 22 系、24 系、または 26 以降 | アプリの画面部分のビルドとテスト |

Node.js のバージョンは、依存しているビルドツールの Vite と、テストツールの Vitest の両方が求める範囲から決めています。23 系と 25 系は Vitest の範囲に入りません。アプリ本体は Tauri v2（Web の技術で画面を作り、Rust で OS の機能を呼ぶデスクトップアプリの枠組み）で作られています。

アプリは利用量の API（アプリが外部のサービスを呼び出すための窓口）との通信の暗号化に、macOS と Linux では OS の TLS（通信を暗号化する仕組み）の実装を、Windows では Rust で書かれた実装の rustls を使います。Linux では、そのための Rust のライブラリ native-tls が OpenSSL を使うので、ビルドの前に OpenSSL の開発用パッケージ（Debian や Ubuntu では `libssl-dev`、Fedora では `openssl-devel`）を入れてください。macOS では OS に含まれる実装を使うので、追加で入れるものはありません。Windows では rustls が使う暗号のライブラリ ring が C のコードを含みますが、Tauri のビルドに要る Visual Studio の C++ のビルドツールでそのままビルドできます。

### ビルドの成果物

`cargo build --release -p marimo-hook` でできる実行ファイルは `target/release/marimo-hook`（Windows では `marimo-hook.exe`）に置かれます。`npm run tauri -- build` は、`app/src-tauri/tauri.conf.json` の `beforeBuildCommand` に従って先に `npm run build` を実行します。macOS では `target/release/bundle/macos/marimo.app` ができます。Windows では、実行ファイル `target/release/marimo.exe` と、二つのインストーラ `target/release/bundle/nsis/marimo_<版>_x64-setup.exe` と `target/release/bundle/msi/marimo_<版>_x64_en-US.msi` ができます。`target/` はリポジトリの直下にあり、`app/` の中ではありません。

`target\release\marimo.exe` を直接起動しても動きますが、ふだん使いには向きません。起動している間は実行ファイルが使用中になり、ビルドし直すと上書きに失敗するためです。MSI（Windows に標準で備わるインストーラの形式）のインストーラは、すべての利用者向けに Program Files へ入れるもので、管理者の権限が要ります。

### ad-hoc 署名について

`marimo.app` は、Apple の開発者証明書ではなく ad-hoc 署名（証明書を使わずに、その場で作った署名）で署名され、Apple の公証も受けていません。このため、次のことが起こりえます。

- 初めて開くときに macOS が「開けません」と警告することがあります。その場合は、Finder で `marimo.app` を control キーを押しながらクリックして「開く」を選ぶか、システム設定の「プライバシーとセキュリティ」で「このまま開く」を押します。
- macOS は、キーチェーンの「常に許可」などの許可を署名に結びつけて覚えます。ビルドし直したり、更新用のスクリプトで CI（継続的インテグレーション。push のたびに自動でテストやビルドを行う仕組み）のビルドに入れ替えたりすると署名が変わるので、キーチェーンの確認がもう一度出ます（[キーチェーンの確認](usage.md#キーチェーンの確認)を参照）。

## フックと statusLine の登録

`marimo-hook install` と `uninstall` が、Claude Code と Codex の設定をどう書き換えるかを説明します。登録の手順は、README の [2. フックと statusLine を登録する](../README.md#2-フックと-statusline-を登録する)にあります。

### install が行うこと

`install` は次のことを行います。

1. 自分自身を `~/.marimo/bin/marimo-hook` へコピーします。settings.json にはこのパスが書かれるので、リポジトリの `target/` を消したりビルドし直したりしても、フックは動き続けます。
2. 設定ファイルの `hooks` に、次の 16 のイベントそれぞれについて、`~/.marimo/bin/marimo-hook` に `hook` という引数を渡して呼ぶフックを追加します。タイムアウトは 5 秒です。
   SessionStart、UserPromptSubmit、PreToolUse、PermissionRequest、PermissionDenied、PostToolUse、PostToolUseFailure、Notification、Elicitation、ElicitationResult、Stop、StopFailure、SessionEnd、MessageDisplay、SubagentStart、SubagentStop

   追加するフックは、macOS では次の形になります。`command` には実行ファイルの絶対パスが入り、Windows では `C:\Users\user\.marimo\bin\marimo-hook.exe` のような `\` 区切りのパスになります。

   ```json
   {
     "type": "command",
     "command": "/Users/user/.marimo/bin/marimo-hook",
     "args": ["hook"],
     "timeout": 5
   }
   ```

   `args` を持つフックは exec form と呼ばれ、Claude Code はシェルを通さずに `command` の実行ファイルを直接起動し、`args` をそのまま引数として渡します。シェルを通さないので、パスに空白があっても引用符で囲む必要がなく、シェルの設定ファイルが出力した文字列がフックの出力に混ざることもありません。exec form に対応した Claude Code の版は、README の[動作環境](../README.md#動作環境)に書いています。

   `~/.marimo/bin/marimo-hook hook` という文字列をシェルに渡す形のフックが登録されている状態で `install` を実行すると、同じグループの同じ位置のまま exec form へ書き換えます。

3. statusLine を登録します（[statusLine の包み方](#statusline-の包み方)を参照）。

### statusLine の包み方

statusLine がまだ無ければ `marimo-hook statusline` だけを登録します。すでに自分の statusLine を使っている場合は、元のコマンドを `marimo-hook statusline -- sh -c '<元のコマンド>'` の形で包みます。marimo は受け取った入力をそのまま元のコマンドへ渡し、元のコマンドの出力と終了コードもそのまま返すので、statusLine の見た目は変わりません。statusLine が command 形式でない場合は書き換えません。statusLine には exec form がないので、こちらはシェルを通すコマンドの文字列で登録します。

Windows の Claude Code は、Git Bash が入っていれば Git Bash で、なければ PowerShell で statusLine を実行します。どちらで動くかが環境によって変わり、両者で引用の規則も違うので、引用符を使わずにどちらのシェルでも同じように読める形だけで書きます。marimo-hook のパスは、実行ファイルがホームフォルダの下にあれば `~/.marimo/bin/marimo-hook.exe statusline` のように `~/` で始め、そうでなければ `/` 区切りの絶対パスを書きます。パスに空白などが含まれていてどちらの形でも書けない場合は、statusLine を登録せず、既存の statusLine も包みません。

既存の statusLine は、`bash C:/Users/user/.claude/statusline.sh` のように、引用符や記号を含まない語を空白一つずつで区切って並べたコマンドの場合だけ包みます。このときは `sh -c` を使わず、`~/.marimo/bin/marimo-hook.exe statusline -- bash C:/Users/user/.claude/statusline.sh` のように元のコマンドをそのまま後ろへ続けます。引用符、`$`、`|`、`;`、`\` などを含むコマンドや、`~` で始まる語を含むコマンドは、二つのシェルで読み方が変わるので包まずにそのまま残し、利用制限は「利用制限を API から取得」で出すよう表示します。`bash ~/.claude/statusline.sh` のように `~` で始まる語だけが理由で包めない場合は、`~` を `C:/Users/user` に書き換えてから `install` を実行し直すと包めます。

包んだコマンドの最初の語が `bash` のような名前だけのときは、marimo-hook が環境変数 `PATH` の並びの順に探して起動します。Rust の標準の探し方は `PATH` より先に System32 などを探すので、WSL（Windows の上で Linux を動かす仕組み）を入れていると System32 の `bash.exe` が見つかり、Git Bash や PowerShell が起動するものと違ってしまうためです。

### 既存の設定の守り方

既存の設定は次のように守られます。

- 書き換える前に、同じフォルダへ `settings.json.marimo-backup-<日時>` という名前でバックアップを取ります。日時は UTC（協定世界時）の `YYYYMMDD-HHMMSS` の形で、同じ名前があれば末尾に `-1` などの番号が付きます。設定ファイルがまだ無かった場合は、バックアップを取らずに新しく作ります。
- 既存のフックや marimo に関係しない項目は、キーの順序も含めてそのまま残します。ファイルの権限も引き継ぎます。settings.json がシンボリックリンクなら、リンク先のファイルを書き換えます。
- 何度実行しても結果は同じです。すでに登録されていれば「変更はありません（すでにインストール済みです）」と表示し、settings.json にもバックアップにも手を付けません。ビルドし直したあとに実行すると、`~/.marimo/bin/marimo-hook` だけが新しいものに差し替わります。シェルに渡す形のフックが残っていた場合だけ、exec form へ書き換えます。[更新用のスクリプト](usage.md#更新する)も、アプリを入れ替えたあとに、新しい `marimo-hook` でこの `install` を実行します。
- settings.json が JSON（データを文字で書き表す形式）として読めないときなどは、何も変更せずに理由を表示し、0 以外の終了コードで終わります。

### サブコマンドと設定ファイルの場所

`marimo-hook` のサブコマンドは `hook`、`codex-hook`、`statusline`、`record`、`install`、`uninstall` の六つです。利用者が直接使うのは `install` と `uninstall` だけで、残りは Claude Code や Codex から呼ばれるか、調査のためのものです。`--help` のようなヘルプの表示はなく、知らないサブコマンドを渡すとエラーを一行出して終わります。

対象の設定ファイルは `--settings <パス>` で指定できます。省略すると、環境変数 `CLAUDE_CONFIG_DIR` が設定されていればその下の `settings.json` を、なければ `~/.claude/settings.json` を使います。

### Codex のフック

`CODEX_HOME` が指すフォルダ（設定していなければ `~/.codex`）があると、`install` は Codex のフックの設定ファイル `hooks.json` にも登録します。フォルダが無ければ Codex は入っていないものとみなし、Codex については何も表示しません。登録するイベントは次の 10 個で、タイムアウトは 5 秒です。

SessionStart、UserPromptSubmit、PreToolUse、PermissionRequest、PostToolUse、Stop、Interrupt、SessionEnd、SubagentStart、SubagentStop

Codex のフックには exec form が無く、コマンドは常に利用者のシェルを通して実行されます。このため、macOS では次のように、実行ファイルのパスを必要に応じて単一引用符で囲み、`codex-hook` を続けた文字列で登録します。

```json
{
  "type": "command",
  "command": "/Users/user/.marimo/bin/marimo-hook codex-hook",
  "timeout": 5
}
```

Windows の Codex は、利用者の設定によって PowerShell かコマンドプロンプトでフックを実行し、両者で引用の規則が違います。そこで、実行ファイルのパスに空白や記号が無く、引用符なしでどちらでも同じように読める場合だけ、`\` 区切りのパスのまま登録します。そう書けない場合は、Codex のフックを登録せずにその旨を表示します。

Codex は、管理者が配ったもの以外のフックを、利用者が Codex CLI（端末で動かす Codex のコマンドラインインターフェース）の `/hooks` で内容を確かめて信頼するまで実行しません。信頼はフックの定義から計算した値で記録されます。信頼する操作は、README の [2. フックと statusLine を登録する](../README.md#2-フックと-statusline-を登録する)に書いています。marimo は Codex の信頼の記録がある `config.toml` を書き換えず、信頼を省く起動のオプションも使いません。`install` を同じ場所の実行ファイルでやり直しても `hooks.json` は変わらないので、信頼し直す必要はありません。`MARIMO_HOME` を変えるなどして実行ファイルの場所が変わったときは、もう一度 `/hooks` で信頼します。

`hooks.json` の書き換えでも、settings.json と同じように、書き換える前に `hooks.json.marimo-backup-<日時>` という名前でバックアップを取り、既存のフックや項目はキーの順序も含めてそのまま残します。ファイルが無ければ新しく作ります。

### uninstall が行うこと

`uninstall` も `--dry-run` と `--settings <パス>` を受け付け、書き換える前にバックアップを取ります。Codex のフォルダがあれば、Codex の `hooks.json` からも marimo のフックだけを取り除きます。取り除くのは marimo が足したものだけで、シェルを通す形で登録されたフックも取り除きます。marimo のフックだけが入っていたグループやイベントは丸ごと消し、他のフックと同じイベントに並んでいた場合は他のフックを残します。statusLine は、marimo だけを登録していた場合は取り除き、元のコマンドを包んでいた場合は元のコマンドへ戻します。

## ファイルとデータ

marimo のデータはすべて `~/.marimo` に置かれます。環境変数 `MARIMO_HOME` を設定すると、その場所を代わりに使います。

| パス | 用途 |
| --- | --- |
| `sessions/<session_id>.json` | セッションごとの状態です。フックが書き、SessionEnd で消します。24 時間更新のないファイルは、アプリが消します |
| `sessions/codex-<session_id>.json` | Codex のセッションごとの状態です。Claude Code と Codex の session_id は別々に振られるので、名前に `codex-` を付けて分けます。marimo はどちらのツールのセッションかと ID（セッションに振られる識別子）の組でセッションを見分けるので、ID が重なっても行や既読の記録が混ざることはありません。書き方と消し方は Claude Code のセッションと同じです |
| `rate_limits.json` | 5 時間と 7 日の利用制限です。statusLine と API からの取得の両方がここへ書きます |
| `codex_rate_limits.json` | Codex の利用制限です。Codex のフックが rollout から読んで書きます |
| `display.json` | 表示の設定です。倍率の `scale`、立ち絵を出すかどうかの `show_character`、パネルの行の出し方の `panel_style`（`"detail"` か `"counts"`）、パネルの行の並べ方の `row_order`（始まった順の `"started"`、状態の順の `"status"`、更新の新しい順の `"updated"` のどれか。無いときや知らない値のときは `"started"`）、API からの取得を使うかどうかの `usage_api`、選んだキャラクターの名前の `character`、選んだキャラクターの立ち絵の枠の縦横比（高さを幅で割った値）の `stage_aspect` を持ちます。`stage_aspect` は画面部分が値を知らせるたびに保存し、次の起動では窓を最初からこの縦横比で開くので、起動の直後に窓が動きません。無いときや値が正しくないときは 1.5 を使い、0.25 から 4.0 の範囲に収めます。古い形式の鍵 `panel_mode` が残っている場合は、表示を切り替えて新しい鍵を保存するまで読み替えて使います（`"list"` は立ち絵なしの詳細、`"picture"` は立ち絵ありの件数だけになります）。アイコンから窓を隠したかどうかは保存しません |
| `window.json` | 窓の位置です |
| `acknowledged.json` | 既読にした完了などのきっかけの記録です |
| `dialogue.json` | 利用者が書くセリフの上書きです。marimo はこのファイルを作りません |
| `dialogue.json.unused-default` | marimo の古い版が書き出した既定のセリフと中身が同じだった `dialogue.json` を、起動時に退避したものです |
| `bin/marimo-hook` | `install` がコピーしたフックのコマンドです。Windows では `marimo-hook.exe` です。settings.json はこのパスを指しています |
| `logs/record-<日付>.jsonl` | 調査用のコマンド `marimo-hook record <ラベル>` が、受け取った JSON をそのまま追記するファイルです。このコマンドを自分で登録したときだけできます。日付は UTC で、ファイルは 1 日ごとに分かれます。プロンプトやツールの引数を含みうるので、追記のたびに 7 日より前の日のファイルを消します。日付の無い名前の `logs/record.jsonl` も、7 日を超えて更新がなければ同じときに消します |
| `logs/usage.log` | 「利用制限を API から取得」の問い合わせの状態が変わるたびに、アプリが手元の時刻を付けて一行ずつ追記するファイルです。同じ状態が続く間と、次に問い合わせる時刻だけが変わったときは書きません。トークンと応答の中身は書きません。64 KB を超えたら、古い方を行の区切りで捨てて新しい方の半分だけを残します |
| `.lock` | フックどうしが同時に書き込んでぶつからないようにするためのロックファイルです |

セッションの状態ファイルは、どのツールのセッションかを表す `provider` を持ち、値は `claude` か `codex` です。この項目を持たないファイルは、Claude Code のセッションとして読みます。Codex のセッションでは、コンテキスト使用率の `source` が `codex-rollout` になります。

`CODEX_HOME` を設定している場合は、`~/.codex` の代わりにその場所を使います。

`codex_rate_limits.json` は次の項目を持ちます。

| 項目 | 意味 |
| --- | --- |
| `windows` | 利用制限の窓の配列です。各要素は、窓の長さ（分）の `window_minutes`、使った割合（%）の `used_percentage`、次に戻る時刻（Unix 秒）の `resets_at` を持ちます。Codex が値を送らなかった窓は含めません |
| `plan_type` | Codex が送ってきたプランの種類です |
| `observed_at` | 値を読んだ rollout の行の時刻（Unix ミリ秒）です。これより古い時点の値では書き換えません |
| `updated_at` | marimo がこのファイルを書いた時刻（Unix ミリ秒）です |

状態ファイルは、一時ファイルに書いてから名前を変える方法で書き換えます。この方法だと、読む側が書きかけのファイルを読むことがありません（atomic write、原子的な書き込みと呼ばれます）。書き込みの途中には、`.` で始まり `.tmp` で終わる一時ファイルが一瞬だけ現れます。

`MARIMO_HOME` は、Claude Code から呼ばれるフックと、アプリの両方に同じ値が見えている必要があります。ログイン時の自動起動ではアプリに `MARIMO_HOME` が渡らないので、アプリは `~/.marimo` を読みます。主に、開発中に本来のデータへ触れずに試すための設定です。

## プライバシーとセキュリティ

marimo は Claude Code の作業を一切妨げないことを最優先にしています。フックは状態ファイルへ書き込むだけで、何が失敗しても終了コード 0 で終わり、標準出力には何も出しません。Claude Code はフックの終了コード 2 で操作を止め、一部のフックの標準出力を Claude への文脈に加えるので、marimo がそのどちらも起こさないようにしています。marimo のアプリが起動していなくても、Claude Code の動作には影響しません。

### marimo が読むもの

- **フックの入力** セッションの ID、作業フォルダ、イベント名、ツール名とその引数、応答の最後の文章、通知の種類、エラーの種類、会話ログのパスなどです。
- **フックの環境** どのアプリから起動されたかを知るための環境変数 `__CFBundleIdentifier` と `TERM_PROGRAM`、そして祖先のプロセスの端末の装置名です。macOS で `__CFBundleIdentifier` が無いときは、セッションの開始時とプロンプトを送ったときに、祖先のプロセスの実行ファイルのパスと、それを収めたアプリの `Info.plist` も読みます。Windows では、これに加えて、Claude Code が動いているコンソールのウィンドウのハンドルと、祖先のプロセスのプロセス ID、起動した時刻、実行ファイルのパスを状態ファイルに記録します。どれも行を押してセッションへ移動するときに使います。
- **作業フォルダの `.git`** 作業フォルダから上へたどって `.git` を探し、リポジトリの名前を決めます。`.git` がファイルのとき（git worktree など）は、その中の `gitdir:` の行だけを読みます。git のコマンドは実行しません。作業フォルダが変わったときと、まだリポジトリ名が分かっていないときだけ調べます。
- **会話ログ** PostToolUse と Stop のときに、会話ログの末尾から最大 4 MB を読み、最後の応答のトークン数と、読んだ範囲にあるチャットの題名だけを取り出します。コンテキスト使用率と題名を出すためです。
- **Codex の rollout と session_index.jsonl** Codex のセッションでは、rollout の末尾から最大 4 MB を読んで最後の `token_count` のトークン数と利用制限だけを取り出し、`session_index.jsonl` の末尾から最大 512 KB を読んでそのセッションの題名だけを取り出します。macOS で起動元のアプリが分からないときは、セッションの開始時とプロンプトを送ったときに rollout の先頭から最大 64 KB を読み、最初の行の `originator` だけを取り出します。Codex の `auth.json` と `config.toml` は読みません。
- **statusLine の入力** コンテキスト使用率、利用制限の値、セッションの名前（`session_name`）を取り出します。入力そのものは、元の statusLine のコマンドへそのまま渡します。
- **OAuth のアクセストークン** 「利用制限を API から取得」を有効にしたときだけ読みます。扱いは[トークンの扱い](usage.md#トークンの扱い)のとおりです。
- **アプリのアイコン** パネルの印に使うため、起動したときに一度だけ、Claude と Codex のデスクトップアプリのアイコンを OS から受け取ります。macOS では NSWorkspace に bundle id `com.anthropic.claudefordesktop` と `com.openai.codex` のアプリのアイコンを、Windows では PackageManager にパッケージ `Claude_pzs8sxrjxfjjc` と `OpenAI.Codex_2p2nqsd0c76g0` のロゴを問い合わせ、アプリの中のファイルを直接は開きません。受け取ったアイコンはメモリに置くだけで、ファイルには書きません。

### marimo が書くもの

- アプリとフックは、`~/.marimo` の中のファイルを書きます（[ファイルとデータ](#ファイルとデータ)を参照）。状態ファイルには、実行したコマンド、ファイルのパス、応答の冒頭などの要約が最大 500 文字まで入ります。リポジトリ名と、チャットの題名も最大 200 文字まで入ります。
- `install` と `uninstall` は、Claude Code の settings.json とそのバックアップを書きます。Codex のフォルダがある場合は、Codex の `hooks.json` とそのバックアップも書きます。
- 「ログイン時に起動」を有効にすると、macOS では LaunchAgent の plist を、Windows ではレジストリの値を書きます（場所は[ログイン時に起動する](usage.md#ログイン時に起動する)を参照）。

### marimo が送るもの

marimo は、状態ファイルも会話の内容も、手元のコンピュータの外へ送りません。ネットワークへの通信は、「利用制限を API から取得」を有効にしたときの `api.anthropic.com` への問い合わせだけです。この機能は既定で無効です。

## リポジトリの構成

| パス | 内容 |
| --- | --- |
| `crates/marimo-core` | 状態のモデル、全体の状態の集約、状態ファイルの読み書き、会話ログからのトークン数と題名の読み取り、Codex の rollout と session_index.jsonl の読み取り、リポジトリ名の判定、Windows でセッションのウィンドウを探して前面に出す処理と MSIX（Windows のアプリのパッケージ形式）のパッケージからアプリのロゴを読む処理を持つ Rust のライブラリ。Windows の API を呼ぶコードはすべてここに置きます |
| `crates/marimo-hook` | Claude Code と Codex から呼ばれるコマンド。`hook`、`codex-hook`、`statusline`、`record`、`install`、`uninstall` のサブコマンドを持ちます |
| `app/src-tauri` | Tauri v2 のアプリ本体（Rust 側）。ファイルの監視、窓の制御、メニューバーと通知領域のアイコン、クリックの透過、セッションへの移動、利用量の API の取得、macOS でのアプリのアイコンの読み取りを受け持ちます |
| `app/src` | 画面部分（TypeScript）。フレームワークは使っていません |
| `assets/character/index.json` | 組み込みのキャラクターの一覧です（[自分のキャラクターを作る](customize.md#自分のキャラクターを作る)を参照） |
| `assets/character/koharu` | 小春の素材。24 枚の PNG（画像の形式）、`manifest.json`、既定のセリフの `dialogue.json` |
| `assets/character/clawd` | Clawd の素材。23 枚の PNG、`manifest.json`、既定のセリフの `dialogue.json` |
| `art/koharu` | 小春の素材の元になった画像 |
| `tools/character` | 小春の素材を作るスクリプト `build.py` とその設定 `koharu.json`、Clawd の素材を描くスクリプト `clawd.py`、Python の依存 `requirements.txt` |
| `tools/tray_icon.py` | メニューバーと通知領域のアイコンを描くスクリプト |
| `tools/app_icon.py` | アプリのアイコンを描くスクリプト |
| `tools/update.sh`、`tools/update.ps1` | 取り込み、CI のビルドの取得（`--build` や `-Build` を付けたときは手元でのビルド）、アプリの差し替え、`install`、起動し直しをまとめて行う更新用のスクリプト。`update.sh` は macOS 用、`update.ps1` は Windows 用です |

## テストと検査

Rust 側のテストと静的検査は、リポジトリの直下で実行します。clippy は Rust の静的解析ツール、`cargo fmt --check` は書式が整っているかを確かめるコマンドです。

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --check
```

Windows の API を呼ぶコードは、macOS でも Windows 向けの型検査だけはできます。事前に `rustup target add x86_64-pc-windows-msvc` で対象を加えておきます。アプリ本体は Windows 向けのリソースのコンパイルに llvm-rc が要るので、この検査には含めていません。

```bash
cargo clippy -p marimo-core -p marimo-hook --all-targets --target x86_64-pc-windows-msvc
```

画面部分の型検査、テスト、ビルドは `app/` で実行します。

```bash
npx tsc --noEmit
npm test
npm run build
```

GitHub Actions の CI（`.github/workflows/ci.yml`）は、main への push と pull request のたびに、macOS と Windows の両方で上の検査を走らせます。画面部分のビルドとテスト、clippy、`cargo test --workspace` を実行し、`cargo fmt --check` は macOS だけで実行します。clippy は警告も失敗として扱います。Windows 向けのコードも Windows の上でビルドしてテストするので、上の型検査だけでは分からない食い違いもここで見つかります。

main への push では、両方の OS で検査が通ったあとに、更新用のスクリプトが取ってくるビルドを作って公開します。macOS では GitHub が用意する arm64 の Mac の上で `marimo.app` と `marimo-hook` をビルドし、シンボリックリンクと実行の権限が保たれるよう tar で `marimo-macos-arm64.tar.gz` にまとめます。Windows では x64 の Windows の上で NSIS（Windows 向けのインストーラを作る仕組み）のインストーラと `marimo-hook.exe` をビルドし、`marimo-windows-x64.zip` にまとめます。両方のビルドが成功したときだけ、二つのファイルとそのチェックサムを書いた `SHA256SUMS` を、GitHub のプレリリースとして公開します。プレリリースには、そのコミットを指す `build-<コミットのハッシュの先頭 12 文字>` という名前のタグを付けます。更新用のスクリプトは、この名前でビルドを探します（[更新する](usage.md#更新する)を参照）。失敗した実行をやり直したときは、同じプレリリースのファイルを差し替えます。

公開のあとに、`build-` で始まるプレリリースを作られた順に並べ、新しい 3 件を残して、それより古いものをタグごと消します。`build-` で始まらないリリースとタグには触れません。このため、main が進んでから古いコミットへ戻して更新しようとしても、そのビルドはもう無いことがあります。そのときは `--build` で手元でビルドします。

文書（`.md` のファイル、`docs/` の中のファイル、`LICENSE-` で始まるファイル）だけを変えた push と pull request では、CI を走らせません。文書に加えて、`tools/` の中のスクリプト、`art/` の元絵、`.gitignore` もアプリには入らないので、main への push がそれらだけを変えたときは、検査はしてもビルドはしません。`target` のジョブが、それら以外を変えた最後のコミットを探し、そのコミットのビルドがまだリリースに無いときだけビルドと公開を行います。公開のジョブはそのコミットの名前でタグを付け、更新用のスクリプトも同じ規則でタグを探します。この一覧は `target` のジョブと二つのスクリプトの三か所にあるので、変えるときは一緒に直します。pull request と手動の実行（workflow_dispatch）では検査だけを行い、ビルドも公開もしません。main へ続けて push したときは、新しいコミットのビルドが始まった時点で古いコミットのビルドを止めますが、始まった公開と片付けは最後まで走らせます。リリースを書き換える権限は公開のジョブにだけ渡し、ほかのジョブは読むだけの権限で動きます。CI がビルドした `marimo.app` も、手元でビルドしたものと同じく ad-hoc 署名です（[ad-hoc 署名について](#ad-hoc-署名について)を参照）。

開発中にアプリを動かすときは、`app/` で `npm run tauri -- dev` を使うと、画面部分の開発用サーバー（ポート 1420）を立ち上げてアプリを起動します。本来の `~/.marimo` に触れずに試したいときは、`MARIMO_HOME` を別のフォルダに向けておきます（[ファイルとデータ](#ファイルとデータ)を参照）。

## アイコンを描き直す

アイコンを描くスクリプトは、[素材を作り直す](customize.md#素材を作り直す)の手順で作った Python の仮想環境で実行します。アイコンはビルドのときにアプリへ埋め込まれるので、描き直したらアプリをビルドし直します。

メニューバーと通知領域のアイコンは、`tools/tray_icon.py` が Pillow だけでまりもの形に描きます。実行すると `app/src-tauri/icons/` へ四つの PNG を書き出します。`tray-template.png` は macOS のふだんのアイコン、`tray-color.png` は Windows のふだんのアイコン、`tray-waiting.png` と `tray-error.png` は承認待ちとエラーのときにどちらの OS でも使うアイコンです（見え方は[メニューバーと通知領域のアイコン](usage.md#メニューバーと通知領域のアイコン)を参照）。メニューバーは画像の高さを 18 ポイントに縮めて置くので、どれも 2 倍の画面で等倍になる 36×36 ピクセルにしています。

アプリのアイコンは、`tools/app_icon.py` が水槽の中の顔のあるまりもを描きます。毛並みは乱数で置いた短い線を数万本重ねて描き、乱数の種を固定しているので、何度実行しても同じ画像になります。実行すると `app/src-tauri/icons/` へ `icon.icns`、`icon.ico`、`icon.png`、`32x32.png`、`128x128.png`、`128x128@2x.png` を書き出します。macOS 用の `icon.icns` は、1024×1024 ピクセルの画像の中央 824 ピクセルに角の丸い板を描いて周りを透明にし、Dock に並ぶ他のアプリと大きさを揃えます。Windows と Linux 用の残りのファイルは、同じ絵から板だけを切り出して画像いっぱいに描きます。

どちらもリポジトリの直下で実行します。

```bash
.venv/bin/python tools/tray_icon.py
.venv/bin/python tools/app_icon.py
```

## 仕組みの概要

### データの流れ

データは次の順に流れます。

1. Claude Code がフックを呼ぶと、`marimo-hook hook` が標準入力の JSON を読み、セッションの新しい状態を決めて `sessions/<session_id>.json` に書きます。Codex のフックからは `marimo-hook codex-hook` が呼ばれ、同じ規則で `sessions/codex-<session_id>.json` に書き、利用制限を `codex_rate_limits.json` へ書きます。statusLine から呼ばれた `marimo-hook statusline` は、コンテキスト使用率とセッションの名前をセッションのファイルへ、利用制限を `rate_limits.json` へ書きます。フックは Claude Code の処理を止めてしまうので、ロックを待つのは最大 300 ミリ秒までにしています。
2. アプリはファイルの監視（file watcher。フォルダの中のファイルが変わると OS から知らせを受け取る仕組み）で `sessions/` と `rate_limits.json` の変化を受け取ります。並列のツール呼び出しで書き込みが重なるので、120 ミリ秒静かになるまで待ち、遅くとも 500 ミリ秒で、全セッションをまとめたスナップショットを読みます。
3. Rust 側は、スナップショットの全体の状態に合わせてメニューバーと通知領域のアイコンを替え、スナップショットを `snapshot` という名前のイベントで画面部分へ送ります。アイコンを画面部分に任せないのは、窓を隠している間は画面部分が止まることがあるからです。
4. 画面部分は、既読の記録を加味して全体の状態を決め直し、表情、吹き出し、パネルを描き直します。

### 状態の決め方

サブエージェント（Claude が Agent ツールで起動する別の Claude）の中で発火したフックも、親のセッションと同じ session_id で届きます。marimo は、フックの入力にある `agent_id` の項目でこれを見分けます。hooks のドキュメントの Common input fields によると、この項目はサブエージェントの中で発火したときだけ入ります。`--agent` で起動したセッションでは親の会話のフックにも `agent_type` が入るので、`agent_type` は見分けには使いません。

サブエージェントのフックは、行の名前も、セッション自身の進み具合も書き換えません。worktree で隔離されたサブエージェントが別のフォルダで動いても、行の名前は親のセッションのリポジトリ名やフォルダ名のままです。marimo は SubagentStart と SubagentStop で動いているサブエージェントを覚えておき、一つでも動いている間は、親のセッションの応答が終わった後でも作業中として出します。最後のサブエージェントが終わると完了になり、完了までにかかった時間もそこまでで数えます。サブエージェントがツールの実行許可を求めたときは承認待ちとして出し、許可か拒否が済むと元の表示に戻ります。前景で動かしたサブエージェントを利用者が途中で止めたときは、SubagentStop が届きません。前景のサブエージェントは親の応答が終わる前に必ず終わるので、親の会話の Stop が届いたときに、その入力の `background_tasks`（裏で動き続けている作業の一覧）に載っていないサブエージェントを外します。一覧の `id` が SubagentStart の `agent_id` と同じ値であることは hooks のドキュメントに書かれていないので、一覧のサブエージェントが一つも覚えているものと一致しないときは、何も外しません。サブエージェントが失敗したときなどにも SubagentStop が届くとは限らないので、30 分のあいだ何のイベントも届かないサブエージェントは終わったものとして扱います。

### 題名とコンテキスト使用率の取り方

デスクトップアプリの Code タブでは statusLine が動かないため、marimo はコンテキスト使用率を会話ログ（transcript。Claude Code がセッションごとに書く JSON Lines 形式の記録）から数えます。このときモデルのコンテキストの上限は分かりません。

題名は二つの場所から取ります。CLI では statusLine の入力にある `session_name` を使います。これは `--name` や `/rename` で付けた名前か、それが無ければ Claude Code が自動で付けた題名です。statusLine が呼ばれないデスクトップアプリの Code タブでは、PostToolUse と Stop のときに読む会話ログの末尾から題名の行を探します。この行の形式は Claude Code のドキュメントに載っていません。どちらの場合も、題名が付く前や最初の読み取りの前は名前だけが出ます。

### Codex のフックから記録するもの

Codex のフックから記録するものは次のとおりです。

- **状態** セッションの開始、プロンプトの送信、ツールの実行の前後、実行許可の確認（PermissionRequest）、応答の終了（Stop）、サブエージェントの開始と終了を、Claude Code と同じ状態に対応させます。Codex の質問のツール `request_user_input` は、Claude Code の AskUserQuestion と同じく承認待ちにします。Codex がサンドボックスの外への書き込みなどの権限を `request_permissions` で求めたときは、PermissionRequest が届かないので、このツールの PreToolUse で承認待ちにし、許可か拒否が済んで PostToolUse が届くと作業中へ戻します。利用者がターンを中断したときに届く Interrupt では、作業中や承認待ちから待機へ戻します。
- **作業の要約** シェルのコマンド（Codex はツール名 `Bash` で送ります）、ファイルの編集（`apply_patch`。パッチに書かれたファイルの名前を出します）、MCP（Model Context Protocol。外部のツールを AI につなぐ仕組み）のツール、サブエージェントの起動（`spawn_agent`）、画像の表示（`view_image`）、質問（`request_user_input`）を要約します。それ以外のツールは、ツール名と引数をそのまま短く出します。
- **コンテキスト使用率** Codex が会話ごとに書く記録（rollout。`~/.codex/sessions/` の下の JSON Lines 形式のファイル）の末尾から最後の `token_count` を読み、Codex の画面の下に出る残りの割合と同じ式で計算します。Codex は、システムプロンプトなどで常に使われる 12000 トークンをコンテキストの上限と使用量の両方から引いて割合を出すので、marimo も同じように引き、100 からその残りの割合を引いた値を使用率として記録します。読むのはセッションの開始、PostToolUse、Stop のときだけです。
- **題名** セッションの開始、プロンプトの送信、Stop のときに、`~/.codex/session_index.jsonl` の末尾からそのセッションの最後の `thread_name` を読みます。Codex のデスクトップアプリは、プロジェクトを選ばずに始めた会話ごとに `~/Documents/Codex/<日付>/<最初のプロンプトから作った名前>` という作業フォルダを作ります。このフォルダ名は題名の言い換えでしかないので、行の名前と吹き出しの `{folder}` には、フォルダ名の代わりに題名を使います。
- **利用制限** rollout の同じ `token_count` にある利用制限を、`~/.marimo/codex_rate_limits.json` に書きます。Codex の利用制限の窓は、プランによって 5 時間と 7 日の二つだったり 7 日の一つだけだったりするので、窓の長さを分単位のまま記録します。複数のセッションの rollout はそれぞれ別の時点の値を持つので、すでに記録した値より新しい時点の値だけで書き換えます。

### 起動元のアプリの見分け方

macOS のフックは、どのアプリから起動されたかを環境変数 `__CFBundleIdentifier` で知ります。Codex のデスクトップアプリから動くセッションのように、この変数が無いときは、セッションの開始時とプロンプトを送ったときに、フックの祖先のプロセスを近い順にたどります。アプリの中にある実行ファイルが見つかれば、それを収めた一番外側のアプリの `Info.plist` から bundle id を読んで記録します。Claude Code の CLI を収めた claude.app のように、前面に出せないアプリ（`Info.plist` に `LSBackgroundOnly` を持つもの）は飛ばして、その先の祖先を探します。それでもアプリが分からない Codex のセッションでは、rollout の先頭の `session_meta` にある `originator` が `Codex Desktop` なら、Codex のデスクトップアプリ（`com.openai.codex`）から始めた会話として記録します。この値は Codex の文書には無く、手元の rollout で確かめたものです。

### クリックの透過

クリックの透過は次のように実現しています。窓がクリックを下へ通す状態になると、マウスの移動のイベントも窓に届かなくなります。そこで、画面部分がクリックを受け取る領域（パネルと吹き出しの矩形と、立ち絵の画像のアルファ値から作った横 40 マスの粗い格子。縦のマス数は素材の縦横比で決まり、小春では 40×60）を Rust 側へ送り、Rust 側はカーソルの位置を定期的に読んで、領域の上にあるときだけ窓がクリックを受け取るように切り替えます。カーソルを読む周期は、窓の上にあるとき 40 ミリ秒、外にあるとき 150 ミリ秒です。アイコンから窓を隠している間はカーソルを読まず、500 ミリ秒ごとに窓が出し直されたかだけを確かめます。立ち絵の上にマウスがあるかどうかも、同じ判定のついでに画面部分へ知らせます。

### 利用量の API への問い合わせ

「利用制限を API から取得」を有効にすると、アプリは `https://api.anthropic.com/api/oauth/usage` を呼び、応答の `five_hour` と `seven_day` の使用率とリセットの時刻を、statusLine と同じ形で `rate_limits.json` に保存します。問い合わせの間隔は[API からの取得のしくみ](usage.md#api-からの取得のしくみ)に、トークンの扱いは[トークンの扱い](usage.md#トークンの扱い)に書いています。

このエンドポイントは公開された API ではなく、文書もありません。応答の形が想定と違うときは、保存している利用制限の値を書き換えず、取得失敗として扱って次の周期を待ちます。

要求は 10 秒で打ち切り、暗号化した通信（HTTPS）でしか送らず、リダイレクト（応答が指す別の場所への転送）は追いません。リダイレクト先へトークンを送らないためです。

問い合わせの結果が変わるたびに、アプリは今の状態を画面部分へ知らせ、画面部分は利用制限の行に理由を出します。状態が変わったときは、`logs/usage.log` にも書きます（[ファイルとデータ](#ファイルとデータ)を参照）。
