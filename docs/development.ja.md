# marimo の開発

[English](development.md) | 日本語

marimo のビルド、`install` が変更する内容、データの保存先、仕組みを記載しています。概要と導入の手順は [README](../README.ja.md) を参照してください。

- [ソースからビルドする](#ソースからビルドする)
- [フックと statusLine の登録](#フックと-statusline-の登録)
- [ファイルとデータ](#ファイルとデータ)
- [プライバシーとセキュリティ](#プライバシーとセキュリティ)
- [リポジトリの構成](#リポジトリの構成)
- [テストと検査](#テストと検査)
- [動作確認の状況](#動作確認の状況)
- [リリースを作る](#リリースを作る)
- [アイコンを描き直す](#アイコンを描き直す)
- [仕組みの概要](#仕組みの概要)

## ソースからビルドする

ビルドのコマンドは、README の [1. ビルドする](../README.ja.md#1-ビルドする)に記載しています。

### 必要なもの

| ツール | バージョン | 用途 |
| --- | --- | --- |
| Rust（rustc と cargo） | 1.90 以降（`Cargo.toml` の `rust-version`）、edition 2024 | フックのコマンド `marimo-hook` とアプリ本体のビルド |
| Node.js と npm | Node.js 22.12 以上の 22 系、24 系、または 26 以降 | アプリの画面部分のビルドとテスト |

Node.js のバージョンは、ビルドツールの Vite と、テストツールの Vitest の両方が要求する範囲から決定しています。23 系と 25 系は Vitest の範囲に含まれません。アプリ本体は Tauri v2（Web の技術で画面を作り、Rust で OS の機能を呼ぶデスクトップアプリのフレームワーク）で作成しています。

### ビルドの成果物

`cargo build --release -p marimo-hook` の実行ファイルは `target/release/marimo-hook`（Windows では `marimo-hook.exe`）に作成されます。`npm run tauri -- build` は、`app/src-tauri/tauri.conf.json` の `beforeBuildCommand` に従って先に `npm run build` を実行し、以下を作成します。`target/` はリポジトリの直下にあり、`app/` の中ではありません。

| OS | 成果物 |
| --- | --- |
| macOS | `target/release/bundle/macos/marimo.app` |
| Windows | 実行ファイル `target/release/marimo.exe`、インストーラ `target/release/bundle/nsis/marimo_<バージョン>_x64-setup.exe` と `target/release/bundle/msi/marimo_<バージョン>_x64_en-US.msi` |

`target\release\marimo.exe` を直接起動しても動作しますが、日常の利用には向きません。起動中は実行ファイルが使用中になり、再ビルド時の上書きに失敗するためです。MSI（Windows に標準で備わるインストーラの形式）のインストーラは、すべての利用者向けに Program Files へ導入するもので、管理者の権限が必要です。

### ad-hoc 署名について

`marimo.app` は、Apple の開発者証明書ではなく ad-hoc 署名（証明書を使わずに、その場で作成した署名）で署名されており、Apple の公証も受けていません。このため、初回起動時に macOS が「開けません」と警告することがあります。その場合は、Finder で `marimo.app` を control キーを押しながらクリックして「開く」を選択するか、システム設定の「プライバシーとセキュリティ」で「このまま開く」を押します。

## フックと statusLine の登録

登録の手順は、README の [2. 各ツールに登録する](../README.ja.md#2-各ツールに登録する)に記載しています。

### install が行うこと

`install` は以下を実行します。2. と 3. は Claude Code の設定ファイルへの登録で、`--settings` を付けずに実行して Claude Code の設定のフォルダ（`~/.claude`、または環境変数 `CLAUDE_CONFIG_DIR` が指すフォルダ）が無ければ、Claude Code は導入されていないとみなしてそのことを表示し、2. と 3. を飛ばします。

1. 自分自身を `~/.marimo/bin/marimo-hook` へコピーします。settings.json にはこのパスが記載されるため、リポジトリの `target/` を削除したり再ビルドしたりしても、フックは動作し続けます。Windows では、statusLine を通す `marimo-hook.exe` が動いていると置き換えられないため、今のものを `.marimo-hook.old.<プロセス ID>.exe` という名前へ移してから新しいものを置きます。移したものは、動いていなければ次の `install` で削除します。
2. 設定ファイルの `hooks` に、以下の 16 のイベントそれぞれについて、`~/.marimo/bin/marimo-hook` に引数 `hook` を渡して呼ぶフックを追加します。タイムアウトは 5 秒です。
   SessionStart、UserPromptSubmit、PreToolUse、PermissionRequest、PermissionDenied、PostToolUse、PostToolUseFailure、Notification、Elicitation、ElicitationResult、Stop、StopFailure、SessionEnd、MessageDisplay、SubagentStart、SubagentStop

   追加するフックは、macOS では以下の形になります。`command` には実行ファイルの絶対パスが入り、Windows では `C:\Users\user\.marimo\bin\marimo-hook.exe` のような `\` 区切りのパスになります。

   ```json
   {
     "type": "command",
     "command": "/Users/user/.marimo/bin/marimo-hook",
     "args": ["hook"],
     "timeout": 5
   }
   ```

   `args` を持つフックは exec form と呼ばれ、Claude Code はシェルを経由せずに `command` の実行ファイルを直接起動し、`args` をそのまま引数として渡します。シェルを経由しないため、パスに空白があっても引用符で囲む必要がなく、シェルの設定ファイルが出力した文字列がフックの出力に混ざることもありません。exec form に対応した Claude Code のバージョンは、README の[動作環境](../README.ja.md#動作環境)に記載しています。

   marimo のフックは、場所によらず実行ファイルの名前が `marimo-hook`（Windows では `marimo-hook.exe`）であるもので見分けます。`<marimo-hook のパス> hook` という文字列をシェルに渡す形のフックや、`MARIMO_HOME` を変えて試したときのように別の場所の `marimo-hook` を指すフックが登録されている場合は、今の場所の exec form の一つにまとめます。今の形のフックが無ければ、最初の一つを同じグループの同じ位置のまま書き換え、残りを取り除きます。

3. statusLine を登録します（[statusLine のラップのしかた](#statusline-のラップのしかた)を参照）。
4. Codex のフックを登録します（[Codex のフック](#codex-のフック)を参照）。
5. OpenCode のプラグインを配置します（[OpenCode のプラグイン](#opencode-のプラグイン)を参照）。
6. Antigravity のフックを登録します（[Antigravity のフック](#antigravity-のフック)を参照）。

4. から 6. は、各ツールの設定のフォルダがあるときのみ実行します。フォルダが無ければ、そのツールは導入されていないとみなし、そのツールについて何も表示しません。Hermes Agent は `install` の対象外で、手動で登録します（[Hermes のフック](#hermes-のフック)を参照）。

| ツール | 設定のフォルダ | 登録先 |
| --- | --- | --- |
| Codex | 環境変数 `CODEX_HOME` が指すフォルダ。未設定なら `~/.codex` | `hooks.json` |
| OpenCode | 環境変数 `XDG_CONFIG_HOME` の下の `opencode`。未設定ならホームの `.config/opencode`（どの OS でも同じ） | `plugins/marimo.js` |
| Antigravity | `~/.gemini/config` | `hooks.json` |

### statusLine のラップのしかた

statusLine が未設定なら、`marimo-hook statusline` のみを登録します。既存の statusLine を使用している場合は、元のコマンドを `marimo-hook statusline -- sh -c '<元のコマンド>'` の形でラップします。marimo は受け取った入力をそのまま元のコマンドへ渡し、元のコマンドの出力と終了コードもそのまま返すため、statusLine の表示は変わりません。statusLine が command 形式でない場合は書き換えません。statusLine には exec form が無いため、シェルを経由するコマンドの文字列で登録します。別の場所の `marimo-hook` でラップしたものや、二重にラップしたものは、すべてのラップを外して元のコマンドを取り出し、今の `marimo-hook` で一重にラップし直します。`uninstall` も、すべてのラップを外して元のコマンドへ戻します。

Windows の Claude Code は、Git Bash があれば Git Bash で、無ければ PowerShell で statusLine を実行します。どちらで動作するかは環境によって変わり、両者で引用の規則も異なるため、引用符を使わずに、どちらのシェルでも同じように解釈される形のみで記載します。marimo-hook のパスは、実行ファイルがホームフォルダの下にあれば `~/.marimo/bin/marimo-hook.exe statusline` のように `~/` で始め、そうでなければ `/` 区切りの絶対パスで記載します。パスに空白などが含まれていてどちらの形でも記載できない場合は、statusLine を登録せず、既存の statusLine もラップしません。

既存の statusLine は、`bash C:/Users/user/.claude/statusline.sh` のように、引用符や記号を含まない語を空白一つずつで区切ったコマンドの場合のみラップします。このときは `sh -c` を使わず、`~/.marimo/bin/marimo-hook.exe statusline -- bash C:/Users/user/.claude/statusline.sh` のように元のコマンドをそのまま後ろへ続けます。引用符、`$`、`|`、`;`、`\` などを含むコマンドや、`~` で始まる語を含むコマンドは、二つのシェルで解釈が変わるためラップせずに残し、Claude Code の利用制限が statusLine から届かないことを通知します。`bash ~/.claude/statusline.sh` のように `~` で始まる語のみが原因でラップできない場合は、`~` を `C:/Users/user` に書き換えてから `install` を再実行するとラップできます。

ラップしたコマンドの最初の語が `bash` のような名前のみのときは、marimo-hook が環境変数 `PATH` の順に検索して起動します。Rust の標準の検索は `PATH` より先に System32 などを検索するため、WSL（Windows の上で Linux を動かす仕組み）を導入していると System32 の `bash.exe` が見つかり、Git Bash や PowerShell が起動するものと異なってしまうためです。

### 既存の設定の守り方

`install` と `uninstall` は、Claude Code の settings.json と、Codex と Antigravity の `hooks.json` を以下の規則で書き換えます。

- 書き換える前に、同じフォルダへ `settings.json.marimo-backup-<日時>` や `hooks.json.marimo-backup-<日時>` のように、ファイル名に `.marimo-backup-<日時>` を付けた名前でバックアップを作成します。日時は UTC（協定世界時）の `YYYYMMDD-HHMMSS` の形で、同じ名前があれば末尾に `-1` などの番号が付きます。ファイルが無かった場合は、バックアップを作成せずに新規作成します。
- 既存のフックや marimo に関係しない項目は、キーの順序も含めてそのまま残します。settings.json では、ファイルの権限も引き継ぎ、シンボリックリンクならリンク先のファイルを書き換えます。
- 何度実行しても結果は同じです。登録済みであれば「変更はありません（すでにインストール済みです）」と表示し、ファイルにもバックアップにも手を付けません。再ビルド後に実行すると、`~/.marimo/bin/marimo-hook` のみが新しいものに入れ替わります。シェルに渡す形のフックが残っていた場合のみ、exec form へ書き換えます。[更新用のスクリプト](usage.ja.md#更新する)も、アプリを入れ替えたあとに、新しい `marimo-hook` でこの `install` を実行します。
- 各ツールの設定ファイルは別々に扱います。settings.json が JSON（データを文字で書き表す形式）として読めないときなどは、そのファイルを変更せずに理由を表示し、ほかのツールの処理を続けます。最後に処理できなかったツールをまとめて表示し、0 以外の終了コードで終了します。
- `--dry-run` では、ファイルを変更せずに、追加や削除するフックと statusLine の変更前後のみを表示します。設定ファイルの全体は、`env` に書いたトークンなどを含みうるため表示しません。

### サブコマンドと設定ファイルの場所

`marimo-hook` のサブコマンドは `hook`、`codex-hook`、`hermes-hook`、`opencode-hook`、`antigravity-hook`、`statusline`、`record`、`install`、`uninstall` の九つです。利用者が直接使用するのは `install` と `uninstall` のみで、残りは各ツールから呼ばれるか、調査用です。`help`、`--help`、`-h` を渡すと使い方を表示します。端末から未知のサブコマンドを渡すと、エラーと使い方を表示して 0 以外の終了コードで終了します。各ツールからフックとして呼ばれたとき（標準入力が端末でないとき）は、ツールの作業を止めないよう、未知のサブコマンドでもエラーを一行出力して終了コード 0 で終了します。

対象の設定ファイルは `--settings <パス>` で指定できます。省略すると、環境変数 `CLAUDE_CONFIG_DIR` が設定されていればその下の `settings.json` を、無ければ `~/.claude/settings.json` を使用します。

### Codex のフック

`install` は、Codex のフックの設定ファイル `hooks.json` に以下の 10 のイベントを登録します。タイムアウトは 5 秒です。

SessionStart、UserPromptSubmit、PreToolUse、PermissionRequest、PostToolUse、Stop、Interrupt、SessionEnd、SubagentStart、SubagentStop

Codex のフックには exec form が無く、コマンドは常に利用者のシェルを経由して実行されます。このため macOS では、以下のように、実行ファイルのパスを必要に応じて単一引用符で囲み、`codex-hook` を続けた文字列で登録します。

```json
{
  "type": "command",
  "command": "/Users/user/.marimo/bin/marimo-hook codex-hook",
  "timeout": 5
}
```

Windows の Codex は、利用者の設定によって PowerShell かコマンドプロンプトでフックを実行し、両者で引用の規則が異なります。そこで、実行ファイルのパスに空白や記号が無く、引用符なしでどちらでも同じように解釈される場合のみ、`\` 区切りのパスのまま登録します。そう記載できない場合は、Codex のフックを登録せずにその旨を表示します。

Codex は、管理者が配布したもの以外のフックを、利用者が Codex CLI（ターミナルで動かす Codex のコマンドラインインターフェース）の `/hooks` で内容を確認して信頼するまで実行しません。信頼はフックの定義から計算した値で記録されます。marimo は Codex の信頼の記録がある `config.toml` を書き換えず、信頼を省く起動のオプションも使用しません。同じ場所の実行ファイルで `install` を再実行しても `hooks.json` は変わらないため、信頼し直す必要はありません。`MARIMO_HOME` を変えるなどして実行ファイルの場所が変わったときは、再度 `/hooks` で信頼します。

`hooks.json` は、[既存の設定の守り方](#既存の設定の守り方)の規則で書き換えます。

### Hermes のフック

`install` と `uninstall` は Hermes の設定を変更しません。Hermes の設定ファイル `config.yaml` はコメントの多い YAML で、利用者が記載したフックも同じ `hooks` の下に並ぶため、それを崩さずに書き換える手段を marimo が持っていないためです。以下の手順で手動登録します。

1. Hermes のフォルダ（環境変数 `HERMES_HOME` が指す場所。未設定なら Windows では `%LOCALAPPDATA%\hermes`、ほかの OS では `~/.hermes`）にある `config.yaml` の `hooks` に、以下の 12 のイベントを追加します。`command` には、`install` がコピーした `marimo-hook` の絶対パスと `hermes-hook` を並べます。Hermes はこの文字列をシェルを経由せずに空白で区切って実行するため、パスに空白がある場合は二重引用符で囲みます。

   ```yaml
   hooks:
     on_session_start: &marimo
       - command: 'C:\Users\user\.marimo\bin\marimo-hook.exe hermes-hook'
         timeout: 5
     on_session_reset: *marimo
     on_session_finalize: *marimo
     pre_llm_call: *marimo
     post_llm_call: *marimo
     on_session_end: *marimo
     pre_tool_call: *marimo
     post_tool_call: *marimo
     pre_approval_request: *marimo
     post_approval_response: *marimo
     api_request_error: *marimo
     post_api_request: *marimo
   ```

   `&marimo` と `*marimo` は、同じ内容を一度だけ記載して再利用する YAML の書き方です。すでに `hooks` に同じイベントのフックがある場合は、`*marimo` を使わず、そのイベントの一覧に marimo の項目を一つ追加します。`fail_closed` は付けません。marimo のフックは何が起きても終了コード 0 で終了し、標準出力に何も出力しないため、Hermes の操作を止めることはありません。

2. Hermes は、初めて見るフックを利用者が承認するまで実行しません。承認はイベントとコマンドの組ごとに、Hermes のフォルダの `shell-hooks-allowlist.json` に記録されます。ターミナルで `hermes` を一度起動すると組ごとに確認が表示されるため、それぞれ承認します。`hermes --accept-hooks` で起動すると、まとめて承認します。gateway は確認を表示できないため、承認していないフックは登録されずに読み飛ばされます。承認の状況は `hermes hooks list` で確認できます。

3. Hermes はフックを起動時にのみ読み込むため、gateway を再起動します。

`marimo-hook` の場所が変わったときは、`command` を書き直し、再度承認します。削除するときは、`config.yaml` から marimo の項目を削除し、`hermes hooks revoke '<command の文字列>'` で承認の記録を削除します。

### OpenCode のプラグイン

OpenCode v2 は、設定のフォルダの `plugins/` に置いた JavaScript のファイルを、設定ファイルに記載しなくても起動時に読み込みます。そこで `install` は、OpenCode の設定ファイル（`opencode.jsonc`）を書き換えず、`plugins/marimo.js` にプラグインを一つ配置します。`plugins/` が無ければ作成します。

プラグインは、リポジトリの `crates/marimo-hook/src/opencode_plugin.js` を雛形にして、`install` がコピーした `marimo-hook` の絶対パスを JSON の文字列として埋め込んだものです。JSON の文字列はそのまま JavaScript の文字列として解釈できるため、パスに空白や引用符があっても崩れません。プラグインは `marimo-hook` をシェルを経由せずに起動します。

既存のファイルは以下のように扱います。

- 一行目が `// marimo-opencode-plugin` のファイルを、marimo が作成したものとみなします。同じ名前でこのマーカーの無いファイルがあれば、`install` も `uninstall` もそのファイルに触れず、その旨を表示します。
- 内容が同じなら「変更はありません（すでにインストール済みです）」と表示し、ファイルに手を付けません。marimo が作成したファイルを手動で書き換えていた場合は、次の `install` で元に戻ります。
- `plugins/marimo.js` がシンボリックリンクなら、リンクを残したままリンク先のファイルを書き換えます。
- settings.json と異なり、バックアップは作成しません。書き換えは、一時ファイルに書き込んでから名前を変える方法で行います。

OpenCode はプラグインを起動時にのみ読み込むため、ファイルを作成または削除したときは、`install` と `uninstall` が以下の案内を表示します。

```text
OpenCode は起動したときにプラグインを読み込みます。動いている OpenCode は、バックグラウンドのサービスも含めて起動し直してください（サービスは opencode service restart で起動し直せます）。
```

### Antigravity のフック

agy の利用者ごとのフックは `~/.gemini/config/hooks.json` に記載します。agy の CLI の `/hooks` がこのファイルの項目を一覧に表示することと、Gemini のデスクトップアプリが動かす agy（Hub）がこのファイルのフックを再起動せずに読み直して呼ぶことを確認しています。このファイルの最上位は、フックの名前ごとの設定です。`install` は、ほかの名前の項目に触れず、`marimo` という名前の項目を一つのみ記載します。

```json
{
  "marimo": {
    "PreInvocation": [
      { "type": "command", "command": "/Users/user/.marimo/bin/marimo-hook antigravity-hook PreInvocation", "timeout": 10 }
    ],
    "PreToolUse": [
      { "matcher": "*", "hooks": [{ "type": "command", "command": "/Users/user/.marimo/bin/marimo-hook antigravity-hook PreToolUse", "timeout": 10 }] }
    ],
    "PostToolUse": [
      { "matcher": "*", "hooks": [{ "type": "command", "command": "/Users/user/.marimo/bin/marimo-hook antigravity-hook PostToolUse", "timeout": 10 }] }
    ],
    "Stop": [
      { "type": "command", "command": "/Users/user/.marimo/bin/marimo-hook antigravity-hook Stop", "timeout": 10 }
    ]
  }
}
```

agy はフックの終了まで処理を止めて待つため、状態の決定に使う四つのイベントのみを登録し、時間の上限を 10 秒にしています。Gemini のデスクトップアプリが動かす agy は入力に `hookEventName` を含めないため、イベント名をコマンドの引数で渡します。agy のハンドラには args が無く、`command` を Unix では `sh -c`、Windows では `cmd /c` に記載どおりに渡します。そこで Codex と同じく、Unix ではパスを単一引用符で引用し、Windows では空白や記号を含まない一語のパスのときのみ登録します（[Codex のフック](#codex-のフック)を参照）。

- すべてのハンドラが、どこかに置いた `marimo-hook` の `antigravity-hook` を呼ぶ項目を、イベント名の引数の有無によらず marimo のものとみなします。古い場所の `marimo-hook` を指す marimo の項目は、名前によらず、新しい `marimo` の項目に置き換えます。
- `marimo` という名前で marimo のものではない項目があれば、書き換えずにエラーで終了します。
- それ以外は、[既存の設定の守り方](#既存の設定の守り方)の規則で書き換えます。

agy が起動時にフックを読み込むことをログで確認しているため、ファイルを書き換えたときは、`install` が以下の案内を表示します。

```text
フックを読み込ませるため、動いている agy の CLI と、Gemini のデスクトップアプリを起動し直してください。
```

### uninstall が行うこと

`uninstall` も `--dry-run` と `--settings <パス>` を受け付け、書き換える前にバックアップを作成します。各ツールから取り除くのは marimo が追加したもののみで、シェルを経由する形で登録されたフックや、別の場所の `marimo-hook` を指すフックも取り除きます。Codex、OpenCode、Antigravity は、設定のフォルダがあるときのみ対象です。

| ツール | 取り除くもの |
| --- | --- |
| Claude Code | settings.json の marimo のフック。statusLine は、marimo のみを登録していた場合は削除し、元のコマンドをラップしていた場合は元のコマンドへ戻します |
| Codex | `hooks.json` の marimo のフック |
| OpenCode | marimo が作成した `plugins/marimo.js`。`plugins/` のフォルダは残します |
| Antigravity | `hooks.json` の marimo の項目 |

marimo のフックのみが入っていたグループやイベントは丸ごと削除し、ほかのフックと同じイベントに並んでいた場合は、ほかのフックを残します。Hermes の `config.yaml` は変更しないため、[Hermes のフック](#hermes-のフック)の手順で手動で削除します。

## ファイルとデータ

marimo のデータはすべて `~/.marimo` に保存されます。環境変数 `MARIMO_HOME` を設定すると、その場所を代わりに使用します。

| パス | 用途 |
| --- | --- |
| `sessions/<session_id>.json` | Claude Code のセッションごとの状態。フックが書き込み、SessionEnd で削除します。完了とエラーのセッションは、SessionEnd の `reason` が `clear`、`resume`、`logout`、`prompt_input_exit` のいずれかのときのみ削除し、それ以外では残します。24 時間更新の無いファイル、バックグラウンドへ移した会話の元のファイル、プロセスが終了した待機のセッションのファイルは、アプリが削除します |
| `sessions/<ツール>-<ID>.json` | Claude Code 以外のツールのセッションごとの状態。名前の先頭に `codex-`、`hermes-`、`opencode-`、`antigravity-` を付けます。ID は、Antigravity では会話の ID（`conversationId`）、ほかは session_id です。marimo はツールと ID の組でセッションを区別するため、ID が重なってもセッションや既読の記録が混ざることはありません。書き込みと削除の規則は Claude Code と同じです。ただし、OpenCode で会話を削除したとき（`session.deleted`）は完了とエラーのセッションも削除し、Antigravity のファイルは会話の終わりを通知するフックが無いためアプリが削除します（[Antigravity のフックから記録するもの](#antigravity-のフックから記録するもの)を参照） |
| `rate_limits.json` | Claude Code の 5 時間と 7 日の利用制限。statusLine の入力から、そのセッションに新しい応答が届いていたときのみ書き込みます。アプリも、Claude Code が `~/.claude.json` の `cachedUsageUtilization` に残した値のうち、保存している `updated_at` より新しい時点のものを書き込みます。このときの `updated_at` は、marimo が読んだ時刻ではなく、Claude Code が値を取得した時刻（`fetchedAtMs`）です。`cachedUsageUtilization` は Claude Code の文書に無い項目で、Claude Code 2.1.316 が書き込む形に合わせています |
| `statusline_responses.json` | セッションごとに、statusLine の入力にあった `cost.total_api_duration_ms`（API の応答を待った時間の累計）と、その値が変わった時刻。この値が前回より増えていれば新しい応答が届いたとみなし、`rate_limits.json` を書き換えます。24 時間変わらなかったセッションの記録は、次の書き込みで削除します |
| `codex_rate_limits.json` | Codex の利用制限。Codex のフックが rollout から読んで書き込むほか、再取得の間隔を選択していれば、アプリが `codex app-server` の `account/rateLimits/read` で問い合わせて書き込みます。項目は下表を参照 |
| `antigravity_rate_limits.json` | Antigravity の Gemini のモデルの利用制限。`rate_limits.json` と同じ形で、`five_hour` に 5 時間の、`seven_day` に週の使用率を持ちます。再取得の間隔を選択していれば、アプリが `agy -p /usage --output-format json` の出力のうち、`id` が `gemini-5h` と `gemini-weekly` のバケットの残りの割合（`remaining_fraction`）を使用率に換算して書き込みます。`updated_at` は `agy` を実行した時刻です。この出力の形は文書に無く、agy 1.5.8 の出力に合わせています |
| `display.json` | 表示の設定。項目は下表を参照 |
| `window.json` | ウィンドウの位置と、立ち絵を置いているウィンドウの隅 |
| `acknowledged.json` | 既読にした完了などのトリガーの記録 |
| `dialogue.json` | 利用者が記載するセリフの上書き。marimo はこのファイルを作成しません |
| `dialogue.json.unused-default` | marimo の古いバージョンが書き出した既定のセリフと内容が同じだった `dialogue.json` を、起動時に退避したもの |
| `bin/marimo-hook` | `install` がコピーしたフックのコマンド。Windows では `marimo-hook.exe`。settings.json はこのパスを指しています |
| `logs/agy-usage.log` | アプリが `agy -p /usage` を実行したときの `agy` のログ。`agy` は起動のたびに自分のログのフォルダへ新しいファイルを作成するため、`--log-file` でこのファイルへ向けます。実行のたびに上書きされます |
| `logs/record-<日付>.jsonl` | 調査用のコマンド `marimo-hook record <ラベル>` が、受け取った JSON をそのまま追記するファイル。このコマンドを自分で登録したときのみ作成されます。日付は UTC で、ファイルは 1 日ごとに分かれます。プロンプトやツールの引数を含みうるため、追記のたびに 7 日より前の日のファイルを削除します。日付の無い名前の `logs/record.jsonl` も、7 日を超えて更新が無ければ同時に削除します |
| `.lock` | フックどうしが同時に書き込んで衝突しないためのロックファイル |

`display.json` の項目は以下のとおりです。値が無いときや正しくないときは、右の列の値を使用します。

| 項目 | 内容 | 値が無いとき、正しくないとき |
| --- | --- | --- |
| `language` | 画面、吹き出しの既定のセリフ、作業の要約、`marimo-hook` と更新用のスクリプトの出力の言語。`"ja"` か `"en"`。アプリのほかに、フック、`marimo-hook`、更新用のスクリプトも読みます | なし。OS の言語が日本語なら日本語、それ以外は英語。`"auto"` も同じ |
| `stage_scale` | 立ち絵の倍率（0.6 から 4.0 までの数） | 以前の `scale`（立ち絵の領域の幅を 180 ピクセルとした倍率）を同じ大きさに読み替えて使用し、次に倍率を保存したときに `scale` を削除 |
| `show_character` | 立ち絵を表示するか | |
| `panel_style` | パネルを表示するか。`"detail"`（表示する）か `"collapsed"`（隠す。立ち絵も隠しているときは利用制限の欄だけを残す） | 以前の `"counts"`（件数だけ）は `"collapsed"` に読み替え、次に表示を保存したときに書き換える |
| `row_order` | パネルのセッションの並べ方。`"started"`（開始順）、`"status"`（状態順）、`"updated"`（更新順） | `"started"` |
| `panel_opacity` | パネルの背景の不透明度（0.2 から 1.0 までの数）。文字やアイコンは薄くしない | 0.8。範囲外の数は近い端に寄せる |
| `panel_scale` | パネルの文字と幅の倍率（0.8 から 1.3 までの数） | 1.0。範囲外の数は近い端に寄せる |
| `tint_strength` | セッションの行の状態の色の濃さに掛ける倍率（0.4 から 2.0 までの数） | 1.0。範囲外の数は近い端に寄せる |
| `show_numbers` | コンテキスト使用率と利用制限のパーセントをバーの横にも表示するか | `false` |
| `hide_keeps_limits` | パネルを隠すときも、立ち絵の有無によらず利用制限の欄を残すか | `false` |
| `bubble_waiting`、`bubble_done`、`bubble_error` | 吹き出しを表示する種類（真偽値） | `true` |
| `bubble_seconds` | 吹き出しを自動で閉じるまでの秒数（0 から 3600 までの整数）。0 は押すまで表示し続ける | 0。上限を超える数は上限に寄せる |
| `usage_refresh_minutes` | 利用制限を再取得させる間隔（分。0 から 1440 までの整数）。0 は再取得しない | 0。上限を超える数は上限に寄せる |
| `usage_tools` | 利用制限の欄に値を表示するツール。`claude`、`codex`、`antigravity` の真偽値を持つオブジェクト。`false` のツールには再取得させない | 無いツールや真偽値でない値は `true` |
| `shortcuts` | グローバルショートカットの設定。`focus_urgent` と `toggle_visible` の文字列を持つオブジェクト | どちらも未設定（無効） |
| `character` | 選択したキャラクターの名前 | |
| `stage_aspect` | 選択したキャラクターの立ち絵の領域の縦横比（高さを幅で割った値）。画面部分が値を通知するたびに保存し、次の起動ではウィンドウを最初からこの縦横比で開くため、起動直後にウィンドウが動かない | 1.5。0.25 から 4.0 の範囲に収める |

古い形式のキー `panel_mode` が残っている場合は、表示を切り替えて新しいキーを保存するまで読み替えて使用します（`"list"` は立ち絵なしの詳細、`"picture"` と `"counts"` は立ち絵ありでパネルを隠した表示）。アイコンからウィンドウを隠したかどうかは保存しません。

`codex_rate_limits.json` の項目は以下のとおりです。

| 項目 | 内容 |
| --- | --- |
| `windows` | 利用制限のウィンドウの配列。各要素は、ウィンドウの長さ（分）の `window_minutes`、使用率（%）の `used_percentage`、次にリセットされる時刻（Unix 秒）の `resets_at` を持ちます。Codex が値を送らなかったウィンドウは含めません |
| `plan_type` | Codex が送ってきたプランの種類 |
| `observed_at` | 値を読んだ rollout の行の時刻か、アプリが `codex app-server` に問い合わせた時刻（Unix ミリ秒）。これより古い時点の値では書き換えません |
| `updated_at` | marimo がこのファイルを書き込んだ時刻（Unix ミリ秒） |

セッションの状態ファイルは、どのツールのセッションかを表す `provider`（`claude`、`codex`、`hermes`、`opencode`、`antigravity`）を持ちます。`provider` を持たないファイルは Claude Code のセッションとして読み、その `provider` を知らない以前のバージョンのアプリは、そのファイルを読み飛ばします。どのツールのファイルも、ターンを動かしているプロセスの ID と起動時刻の `host` を持ちます（[プロセスが異常な終わり方をしたとき](#プロセスが異常な終わり方をしたとき)を参照）。Claude Code のファイルは、会話ログの場所の `transcript_path` も持ちます（[バックグラウンドへ移した会話](#バックグラウンドへ移した会話)と[利用者が止めた応答](#利用者が止めた応答)を参照）。ほかのツールでは以下の値を持ちます。

| ツール | コンテキスト使用率の `source` | そのほかの項目 |
| --- | --- | --- |
| Codex | `codex-rollout` | |
| Hermes | `hermes-api` | 現在のターンの ID の `turn_id`、パネルのセッションを押したときに開く URL の `link` |
| OpenCode | `opencode-step` | |
| Antigravity | `antigravity-transcript` | 会話の DB の場所を求めるための `transcript_path` |

`CODEX_HOME` を設定している場合は、`~/.codex` の代わりにその場所を使用します。

状態ファイルは、一時ファイルに書き込んでから名前を変える方法で書き換えます。この方法では、読む側が書き込み途中のファイルを読むことがありません（atomic write、原子的な書き込みと呼ばれます）。書き込みの途中には、`.<ファイル名>.<プロセス ID>.<通し番号>.tmp` という名前の一時ファイルが一瞬だけ作成されます。フックが書き込みの途中で止められて残った一時ファイルは、アプリが起動時と 10 分ごとに、1 時間より古いものを `~/.marimo` の直下と `sessions` の中から削除します。Windows では、ウイルス対策ソフトなどがファイルを一瞬開いていると名前の変更が失敗するため、間隔を倍にしながら合わせて 0.5 秒ほどまで再試行します。

`MARIMO_HOME` は、各ツールから呼ばれる `marimo-hook` と、アプリの両方に同じ値が見えている必要があります。OpenCode では、プラグインを動かしているサービスのプロセスに見えている値が `marimo-hook` に渡ります。ログイン時の自動起動ではアプリに `MARIMO_HOME` が渡らないため、アプリは `~/.marimo` を読みます。主に、開発中に本来のデータへ触れずに試すための設定です。

## プライバシーとセキュリティ

marimo は、対応しているツールの作業を一切妨げないことを最優先にしています。フックは状態ファイルへ書き込むのみで、何が失敗しても終了コード 0 で終了し、標準出力には何も出力しません。Claude Code はフックの終了コード 2 で操作を止め、一部のフックの標準出力を Claude への文脈に加えます。Codex と Hermes のフックも、終了コードと標準出力で操作を止めたり会話に文脈を加えたりできます。marimo はそのいずれも起こさないようにしています。OpenCode のプラグインは `marimo-hook` の終了を待たずに OpenCode の処理を続け、`marimo-hook` の終了コードと出力も読みません。agy の文書はフックに JSON の出力を求めますが、`antigravity-hook` もほかと同じく何も出力しません。agy 1.5.8 が、何も出力しないフックを失敗として記録しないことと、承認が必要な操作の扱いがフックの有無で変わらないことを確認しています。marimo のアプリが起動していなくても、どのツールの動作にも影響しません。

### marimo が読むもの

- **フックの入力** セッションの ID、作業フォルダ、イベント名、ツール名とその引数、応答の最後の文章、通知の種類、エラーの種類、会話ログのパスなど。
- **フックの環境** どのアプリから起動されたかを知るための環境変数 `__CFBundleIdentifier` と `TERM_PROGRAM`、デスクトップアプリの Code タブの会話の id を表す `CLAUDE_CODE_HOST_SESSION_ID`、祖先のプロセスのターミナルの装置名。macOS で `__CFBundleIdentifier` が無いときは、セッションの開始時とプロンプトの送信時に、祖先のプロセスの実行ファイルのパスと、それを含むアプリの `Info.plist` も読みます。Windows では、これに加えて、Claude Code か Codex が動作しているコンソールのウィンドウのハンドルと、祖先のプロセスのプロセス ID、起動時刻、実行ファイルのパスを状態ファイルに記録します。いずれもパネルのセッションを押して移動するときに使用します。Hermes と OpenCode のセッションでは、フックの祖先が会話のウィンドウではないため、これらを読みません。
- **作業フォルダの `.git`** 作業フォルダから上へたどって `.git` を検索し、リポジトリの名前を決定します。`.git` がファイルのとき（git worktree など）は、その中の `gitdir:` の行のみを読みます。git のコマンドは実行しません。作業フォルダが変わったときと、リポジトリ名がまだ分からないときのみ調べます。
- **会話ログ** PostToolUse と Stop のときに、会話ログの末尾から最大 4 MB を読み、最後の応答のトークン数と、読んだ範囲にあるチャットのタイトルのみを取り出します。コンテキスト使用率とタイトルを表示するためです。
- **Codex の rollout と session_index.jsonl** Codex のセッションでは、rollout の末尾から最大 4 MB を読んで最後の `token_count` のトークン数と利用制限のみを取り出し、`session_index.jsonl` の末尾から最大 512 KB を読んでそのセッションのタイトルのみを取り出します。利用者が自分で始めた会話かを区別するため、フックが届くたびに rollout の先頭から最大 64 KB を読み、最初の行の `originator` のみを取り出します。Codex の `auth.json` と `config.toml` は読みません。
- **Hermes のセッションの一覧** Hermes のセッションでは、ターンの始まりに、Hermes のフォルダの `sessions/sessions.json` からそのセッションの行を検索し、チャットの種類と名前、Discord のサーバーとチャンネルの ID のみを取り出します。Hermes の `.env`、`auth.json`、`state.db` は読みません。
- **OpenCode のプラグインが渡すもの** OpenCode のセッションでは、プラグインが選択したイベントの内容（セッションの ID、作業フォルダ、ツールの名前と引数、実行許可を求める操作の種類と対象、質問のフォームのフィールド、エラーの種類と文、各ステップのトークン数）を受け取ります。プラグインはこれに、ターンで最後に書かれた応答の文章と、OpenCode に問い合わせた値を追加して渡します。プラグインが OpenCode に問い合わせるのは、作成を見ていないセッションの親、タイトル、作業フォルダと、モデルの一覧にあるコンテキストの上限のみです。OpenCode のファイルは読みません。
- **ターンを動かしているプロセス** セッションの開始時とプロンプトの送信時（Hermes と OpenCode ではターンの始まり）と、まだ記録の無いセッションへのフックで、フックの祖先のうちシェルでない最も近いプロセスの ID と起動時刻を状態ファイルに記録します。シェルを区別するために祖先の実行ファイルの名前を読みますが、名前は記録しません。アプリは、記録したプロセスが動作しているかを OS に問い合わせるのみで、引数は読みません。
- **Antigravity のフックの入力と transcript** Antigravity のセッションでは、フックの入力から会話の ID、作業フォルダ、ツールの名前と引数、ターンの終わりの理由を取り出します。Gemini のデスクトップアプリの会話では、ターンの始まりと終わりに `annotations/<会話の ID>.pbtxt` からタイトルのみを読みます。フックが届くたびに、会話の transcript（agy が `brain/<会話の ID>/.system_generated/logs/transcript_full.jsonl` に書き込むもの）の先頭から 2 KB と、末尾から最後のモデルの応答の行までを読み、サブエージェントの会話かどうかと、最後の応答の文章と、最後のモデルの呼び出しの入力のトークン数のみを取り出します。agy の資格情報と会話の一覧のデータベースは読みません。
- **Antigravity の会話の DB** 作業中か承認待ちの Antigravity のセッションがある間、アプリが 1 秒ごとに、その会話と、そのセッションの中で動作しているサブエージェントの会話の DB（agy のデータのフォルダの `conversations/<会話の ID>.db`）を読み取り専用で開き、最後の step の状態、ツールの名前と引数、状態が変わった時刻、現在のターンに終わっていない step があるかのみを取り出します。承認待ち、承認を断って止まったターン、モデルの応答を待つ間に止めたターンを区別するためです（[Antigravity のフックから記録するもの](#antigravity-のフックから記録するもの)を参照）。
- **statusLine の入力** コンテキスト使用率、利用制限の値、セッションの名前（`session_name`）、API の応答を待った時間の累計（`cost.total_api_duration_ms`）を取り出します。入力そのものは、元の statusLine のコマンドへそのまま渡します。statusLine は応答が届いたときのほかに、prompt cache（会話の前半を API 側に一時的に保存しておく仕組み）が期限を迎えたときにも呼ばれ、そのときの入力にはそのセッションが最後に応答を受け取ったときの古い利用制限が入っています。このため、利用制限は、前回の呼び出しから新しい応答が届いていたときのみ保存します（`statusline_responses.json` を参照）。
- **Claude Code の `~/.claude.json`** アプリが 30 秒ごとに更新時刻を確認し、変わっていたら読んで、Claude Code が取得した利用制限の `cachedUsageUtilization` と、ログイン中のアカウントの `oauthAccount.accountUuid` のみを取り出します。アカウントの ID は、別のアカウントで取得された値を使わないために照合するのみで、記録しません。Claude Code のトークンを含む資格情報は読みません。
- **OS の言語** 「言語」を選択していないとき、表示の言語を決めるために、macOS では優先する言語の一覧の先頭を、Windows では表示言語を OS に問い合わせます。
- **アプリのアイコン** パネルのマークに使用するため、起動時に一度だけ、Claude、Codex、OpenCode、Gemini のデスクトップアプリのアイコンを OS から受け取ります。macOS では NSWorkspace に bundle id `com.anthropic.claudefordesktop`、`com.openai.codex`、`ai.opencode.desktop`、`com.google.GeminiMacOS`（Antigravity を同梱しているアプリ）のアプリのアイコンを、Windows では PackageManager でパッケージ `Claude_pzs8sxrjxfjjc` と `OpenAI.Codex_2p2nqsd0c76g0` のアプリの ID を調べ、スタートメニューの一覧と同じアイコンをシェル（`shell:AppsFolder`）から受け取ります。パッケージが返すロゴ（`GetLogo`）はタイル用で周りの余白が大きく、パネルでは小さくなりすぎるため使いません。どちらの OS でも、アプリの中のファイルを直接は開きません。Windows の Gemini のデスクトップアプリ（Antigravity を同梱しているアプリ）は MSIX ではないため、`%LOCALAPPDATA%\Google\Gemini\Gemini.exe` のアイコンを、エクスプローラーが表示するものと同じサムネイルとしてシェルから受け取り、PNG に変換します。Windows では OpenCode のアイコンを問い合わせません。受け取ったアイコンはメモリに保持するのみで、ファイルには書き込みません。Hermes Agent のアイコンと、OpenCode のデスクトップアプリのアイコンを受け取れなかったときの OpenCode のアイコンは、`app/src/assets/providers/` に同梱した画像を使用します（出典と許諾は `LICENSE-ASSETS` に記載しています）。

### marimo が起動するもの

- **`codex app-server`** 設定のウィンドウの「再取得の間隔」を選択したときのみ、保存している Codex の値がその間隔より古ければ起動し、`initialize` のあとに `account/rateLimits/read` のみを送り、応答を受け取ったら標準入力を閉じて終了させます。Codex への認証は Codex 自身が行い、marimo は Codex の `auth.json` を読みません。会話は始めないため rollout は作成されません。
- **`claude -p "/usage" --no-session-persistence --settings '{"disableAllHooks": true}'`** 設定のウィンドウの「再取得の間隔」を選択したときのみ、保存している Claude の値がその間隔より古ければ、`~/.marimo` を作業フォルダにして起動します。Claude Code は、セッションが動作しているだけでは `~/.claude.json` の利用制限を再取得しないためです。`/usage` は Claude Code の中で処理され、モデルへのリクエストは送られません。`--no-session-persistence` で会話ログを残さず、`disableAllHooks` で marimo を含むフックを動作させません。60 秒で終了しなければ止めます。
- **`agy -p /usage --output-format json --log-file ~/.marimo/logs/agy-usage.log`** 設定のウィンドウの「再取得の間隔」を選択したときのみ、保存している Antigravity の値がその間隔より古ければ、`~/.marimo` を作業フォルダにして起動します。`/usage` は agy の中で処理され、会話を作成せず、モデルへのリクエストも送りません。Antigravity への認証は agy 自身が行い、marimo は agy の資格情報を読みません。60 秒で終了しなければ止めます。

いずれのコマンドも、設定のウィンドウの「利用制限」でそのツールのチェックを外していれば起動しません。また、そのツールで作業中で新しい値が届いている間は起動しません。GUI のアプリはターミナルの PATH を引き継がないため、コマンドは PATH のほかに `~/.local/bin`、macOS では `/opt/homebrew/bin` と `/usr/local/bin` からも検索し、`codex` は macOS の ChatGPT のデスクトップアプリが同梱する `Contents/Resources/codex-cli/bin` からも検索します。npm で導入した `codex` は `node` で動作するため、見つけたコマンドのフォルダを子プロセスの PATH の先頭に追加します。

### marimo が書き込むもの

- アプリとフックは、`~/.marimo` の中のファイルに書き込みます（[ファイルとデータ](#ファイルとデータ)を参照）。状態ファイルには、実行したコマンド、ファイルのパス、応答の冒頭などの要約が最大 500 文字まで入ります。リポジトリ名と、チャットのタイトルも最大 200 文字まで入ります。
- `install` と `uninstall` は、Claude Code の settings.json とそのバックアップに書き込みます。Codex と Antigravity の設定のフォルダがある場合は、それぞれの `hooks.json` とそのバックアップにも書き込みます。OpenCode の設定のフォルダがある場合は、`plugins/marimo.js` を作成するか削除します。
- 「ログイン時に起動」を有効にすると、macOS では LaunchAgent の plist を、Windows ではレジストリの値を書き込みます（場所は[ログイン時に起動する](usage.ja.md#ログイン時に起動する)を参照）。

### marimo が送るもの

marimo は、状態ファイルも会話の内容も、ローカルのコンピュータの外へ送りません。フックはネットワークへの通信をしません。アプリが通信するのは GitHub のみで、起動の 3 秒後、1 日ごと、「更新を確認」を押したときにリリースの一覧を API で読み、更新するときにリリースのファイルを取得します（[更新を確認する](usage.ja.md#更新を確認する)を参照）。いずれも送るのはリクエストのみで、marimo の状態は含みません。吹き出しのセリフは用意した候補から選ぶため、大規模言語モデル（LLM）は呼びません。OpenCode のプラグインは、OpenCode のプラグインの仕組みを通して OpenCode に問い合わせることと、`marimo-hook` を起動して標準入力にイベントを渡すことのみを行います。

## リポジトリの構成

| パス | 内容 |
| --- | --- |
| `crates/marimo-core` | 状態のモデル、全体の状態の集約、状態ファイルの読み書き、各ツールの入力の読み替えと記録の読み取り（Claude Code の会話ログ、Codex の rollout と session_index.jsonl、Hermes のセッションの一覧、OpenCode のイベント、Antigravity の transcript）、ターンを動かしているプロセスの生存確認、リポジトリ名の判定、Windows でセッションのウィンドウを検索して前面に表示する処理、MSIX（Windows のアプリのパッケージ形式）のアプリと実行ファイルのアイコンを読む処理、表示の言語の決定を持つ Rust のライブラリ。Windows の API を呼ぶコードはすべてここに置きます |
| `crates/marimo-hook` | 各ツールから呼ばれるコマンド。サブコマンドは[サブコマンドと設定ファイルの場所](#サブコマンドと設定ファイルの場所)を参照。`install` が配置する OpenCode のプラグインの雛形 `src/opencode_plugin.js` もここにあり、ビルド時に実行ファイルへ埋め込まれます |
| `app/src-tauri` | Tauri v2 のアプリ本体（Rust 側）。ファイルの監視、Claude Code、Codex、Antigravity からの利用制限の取得、ウィンドウの制御、メニューバーと通知領域のアイコン、クリックの透過、セッションへの移動、macOS でのアプリのアイコンの読み取りを担当します |
| `app/src` | 画面部分（TypeScript）。フレームワークは使用していません |
| `assets/character/index.json` | 組み込みのキャラクターの一覧（[自分のキャラクターを作る](customize.ja.md#自分のキャラクターを作る)を参照） |
| `assets/character/koharu` | 小春の素材。32 枚の PNG、`manifest.json`、既定のセリフの `dialogue.json` と英語の `dialogue.en.json` |
| `assets/character/clawd` | Clawd の素材。22 枚の PNG、`manifest.json`、既定のセリフの `dialogue.json` と英語の `dialogue.en.json` |
| `art/koharu` | 小春の素材の元になった画像 |
| `tools/character` | 小春の素材を作成するスクリプト `build.py` とその設定 `koharu.json`、Clawd の素材を描くスクリプト `clawd.py`、Python の依存 `requirements.txt` |
| `tools/tray_icon.py` | メニューバーと通知領域のアイコンを描くスクリプト |
| `tools/app_icon.py` | アプリのアイコンを描くスクリプト |
| `tools/release.sh` | リリースを作成する macOS 用のスクリプト（[リリースを作る](#リリースを作る)を参照） |
| `tools/update.sh`、`tools/update.ps1` | 更新用のスクリプト。`update.sh` は macOS 用、`update.ps1` は Windows 用（[更新する](usage.ja.md#更新する)を参照） |

## テストと検査

Rust 側のテストと静的検査は、リポジトリの直下で実行します。clippy は Rust の静的解析ツール、`cargo fmt --check` は書式を確認するコマンドです。

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --check
```

Windows の API を呼ぶコードは、macOS でも Windows 向けの型検査のみ実行できます。事前に `rustup target add x86_64-pc-windows-msvc` で対象を追加しておきます。アプリ本体は Windows 向けのリソースのコンパイルに llvm-rc が必要なため、この検査には含めていません。

```bash
cargo clippy -p marimo-core -p marimo-hook --all-targets --target x86_64-pc-windows-msvc
```

画面部分の型検査、テスト、ビルドは `app/` で実行します。

```bash
npx tsc --noEmit
npm test
npm run build
```

立ち絵、パネル、吹き出しの配置は、`npm run test:visual` で確認します。Playwright（ブラウザを操作するテストツール）が headless の Chromium（画面にウィンドウを表示しないブラウザ）で `index.html` を開き、Tauri の `invoke` と `listen` を `@tauri-apps/api/mocks` の mock に置き換えて、四つの隅の配置と、立ち絵を押したときにパネルを隠す切り替えを確認します。画面には何も表示されないため、作業中の画面の邪魔になりません。各隅の画面写真は `app/test-results/` に保存されます。初回は `npx playwright install chromium` でブラウザを取得しておきます。CI ではこのテストを実行していません。

ウィンドウを置く位置と隅の選び直し（`app/src-tauri/src/window_pos.rs`）は、ウィンドウの操作を `Screen` という trait（Rust の interface にあたる仕組み）の後ろに置いているため、`cargo test` の中で画面を持たない fake のウィンドウを使い、起動から保存までの流れを確認しています。macOS はウィンドウの大きさと位置の変更をあとで適用し、直後に読み返すと変更前の値が返るため、fake も同じように振る舞います。

開発中にアプリを動かすときは、`app/` で `npm run tauri -- dev` を使うと、画面部分の開発用サーバー（ポート 1420）を起動してからアプリを起動します。本来の `~/.marimo` に触れずに試すときは、`MARIMO_HOME` を別のフォルダに向けておきます（[ファイルとデータ](#ファイルとデータ)を参照）。

### CI

GitHub Actions の CI（`.github/workflows/ci.yml`）は、main への push と pull request のたびに、macOS と Windows の両方で上の検査を実行します。画面部分のビルドとテスト、clippy、`cargo test --workspace` を実行し、`cargo fmt --check` は macOS のみで実行します。clippy は警告も失敗として扱います。Windows 向けのコードも Windows の上でビルドしてテストするため、上の型検査のみでは分からない食い違いもここで見つかります。

main への push では、両方の OS で検査が通ったあとに、更新用のスクリプトが取得するビルドを作成して公開します。macOS では GitHub が用意する arm64 の Mac の上で `marimo.app` と `marimo-hook` をビルドし、シンボリックリンクと実行の権限が保たれるよう tar で `marimo-macos-arm64.tar.gz` にまとめます。Windows では x64 の Windows の上で NSIS（Windows 向けのインストーラを作成する仕組み）のインストーラと `marimo-hook.exe` をビルドし、`marimo-windows-x64.zip` にまとめます。両方のビルドが成功したときのみ、二つのファイルとそのチェックサムを記載した `SHA256SUMS` を、GitHub のプレリリースとして公開します。プレリリースには、そのコミットを指す `build-<コミットのハッシュの先頭 12 文字>` という名前のタグを付けます。更新用のスクリプトは、`--main` や `--no-pull` を付けたときに、この名前でビルドを検索します（[更新する](usage.ja.md#更新する)を参照）。失敗した実行を再実行したときは、同じプレリリースのファイルを差し替えます。

公開のあとに、`build-` で始まるプレリリースを作成順に並べ、新しい 3 件を残して、それより古いものをタグごと削除します。この処理は `v` で始まるリリースとタグには触れません。main のビルドは古いものから削除されるため、main が進んだあとに古いコミットへ戻して更新しようとしても、そのビルドが無いことがあります。そのときは `--build` でローカルでビルドします。

文書（`.md` のファイル、`docs/` の中のファイル、`LICENSE-` で始まるファイル）のみを変更した push と pull request では、CI を実行しません。GitHub はタグの push にこの絞り込みを適用しないため、`v` で始まるタグの push では常に実行します。文書に加えて、`tools/` の中のスクリプト、`art/` の元絵、`.gitignore`、`.github/` の中の CI の設定もアプリに含まれないため、main への push がそれらのみを変更したときは、検査はしてもビルドはしません。`target` のジョブが、それら以外を変更した最後のコミットを検索し、そのコミットのビルドがまだリリースに無いときのみビルドと公開を行います。公開のジョブはそのコミットの名前でタグを付け、更新用のスクリプトも同じ規則でタグを検索します。この対象の一覧は `target` のジョブと二つのスクリプトの三か所にあるため、変更するときは一緒に修正します。

pull request と、main と `v` で始まるタグ以外での手動の実行（workflow_dispatch）では検査のみを行い、ビルドも公開もしません。main での手動の実行は、main への push と同じく、ビルドが無ければビルドして公開します。`v` で始まるタグを選択した手動の実行は、タグの push と同じく正式なリリースを作成します（[リリースを作る](#リリースを作る)を参照）。内容の変わらないコミットへ force push すると、push では CI が起動しないため、そのコミットのビルドはこの手動の実行で作成します。main へ続けて push したときは、どのコミットのビルドも止めずに最後まで実行します。後から始まったビルドで先のビルドを止めると、古いコミットの検査が遅れて終わったときに新しいコミットのビルドを止めてしまい、`tools/release.sh` が待つバージョンのコミットのビルドが無くなるためです。リリースを書き換える権限は公開のジョブのみに渡し、ほかのジョブは読み取りの権限で動作します。CI がビルドした `marimo.app` も、ローカルでビルドしたものと同じく ad-hoc 署名です（[ad-hoc 署名について](#ad-hoc-署名について)を参照）。

## 動作確認の状況

ツールと画面ごとに、確認した内容と制約をまとめています。必要なバージョンは、README の[動作環境](../README.ja.md#動作環境)に記載しています。

| ツール | 使い方 | 確認の状況 | 確認した内容と制約 |
| --- | --- | --- | --- |
| Claude Code | macOS のターミナルの CLI | 確認済み | すべての機能。フックと statusLine の両方の動作と、更新用のスクリプトによるローカルでのビルドと更新を確認しています。パネルのセッションを押して移動する機能は Terminal.app と Ghostty で確認しており、Ghostty では Claude Code の background session からも移動できます。iTerm2 と VS Code の統合ターミナルでの区別のしかたは、それぞれのアプリの文書に基づいています |
| Claude Code | macOS のデスクトップアプリの Code タブ | 確認済み | フックは承認待ちを含めて届きます。statusLine は呼ばれません（[利用制限の表示](usage.ja.md#利用制限の表示)を参照） |
| Claude Code | VS Code の拡張機能 | 未確認 | statusLine が動作するかを確認していません |
| Claude Code | Windows | 一部確認済み | Windows 11 で、ビルド、NSIS のインストーラによる導入、更新用のスクリプトによるローカルでのビルドと更新、`install`、フックの発火、自動起動、クリックの透過、通知領域のアイコンからウィンドウを隠して再表示する操作を確認しています。statusLine が無い場合の `install` による登録が Git Bash と PowerShell のどちらでも動作することと、既存の statusLine を `install` がラップしても元の表示が変わらず、CLI のセッションでコンテキスト使用率がパーセントで表示されることも確認しています（[statusLine のラップのしかた](#statusline-のラップのしかた)を参照）。パネルのセッションを押して移動する機能は、デスクトップアプリの Code タブ、Windows Terminal、conhost、VS Code の統合ターミナルのいずれでも確認しています（[セッションへ移動する](usage.ja.md#セッションへ移動する)を参照） |
| Claude Code | Linux | 未確認 | 動作を確認していません |
| Claude Code | クラウドで動くセッション（スマートフォンの Code タブなど） | 対象外 | ローカルの `~/.claude/settings.json` を読まないため、フックが届きません |
| Claude Code | Cowork | 対象外 | settings.json のフックが発火しないという報告があります |
| Codex | macOS のデスクトップアプリ | 確認済み | `/hooks` で信頼したフックが届き、作業中、`request_permissions` による承認待ち、完了、タイトル、コンテキスト使用率がパネルに表示されることと、セッションを押すとデスクトップアプリが前面に表示されること、利用制限が `codex_rate_limits.json` に記録されることを確認しています（[Codex のフック](#codex-のフック)を参照） |
| Codex | macOS のターミナルの Codex CLI | 確認済み | Terminal.app で、フックが届いて作業中と承認待ちが表示されることと、セッションを押すとターミナルが前面に表示されることを確認しています |
| Codex | Windows の Codex CLI | 一部確認済み | フックが届いてパネルのセッションの状態が変わることと、アプリのアイコンが表示されることを確認しています。フックを登録できる実行ファイルのパスの条件は[Codex のフック](#codex-のフック)を参照 |
| Hermes Agent | Windows の Discord の gateway | 一部確認済み | 手動で登録したフックが gateway に読み込まれ、Discord のチャンネルでの会話が、チャンネル名のセッションとして作業中から完了へ移ることと、応答のあとに Hermes がバックグラウンドで実行する見直しで完了のセッションが作業中へ戻らないことを確認しています。セッションを押すと Discord のそのチャンネルが開き、Discord のウィンドウが前面に表示されることも確認しています。承認待ち、エラー、CLI の会話は確認していません（[Hermes のフック](#hermes-のフック)を参照） |
| OpenCode | macOS のターミナルの v2 | 一部確認済み | `opencode run --standalone` で、作業中から完了、許可待ちから作業中、質問待ちから作業中へ移ることと、サブエージェントが自分のセッションをパネルに表示しないこと、タイトルとコンテキスト使用率が表示されることを確認しています。TUI とバックグラウンドのサービスの下でも、セッションが表示されて作業中、許可待ち、完了と移ることと、セッションを押すと Ghostty のターミナルへ移動できること、中断で待機に戻ることと、エラーが表示されることを確認しています |
| OpenCode | Windows のターミナルの v2 の TUI | 未確認 | 動作を確認していません |
| OpenCode | v1 のデスクトップアプリ | 対象外 | marimo のプラグインは v2 のプラグインの仕組みで作成しているため、v1 では読み込まれません |
| Antigravity | macOS のターミナルの agy 1.5.8 の CLI | 一部確認済み | ワークスペースの `.agents/hooks.json` に同じフックを置き、`agy -p` で、作業中から完了へ移ることと、作業中の要約にコマンドの説明（`toolSummary`）が表示されること、コンテキストのトークン数が表示されること、起動元に Ghostty が記録されること、サブエージェントが自分のセッションをパネルに表示せずに親のセッションの中で数えられること、親のセッションがサブエージェントの終了まで作業中のままでいること、Claude Code の中から起動した agy がパネルに表示されないこと、承認が必要な操作（ワークスペース外への書き込み）の扱いがフックの有無で変わらないことを確認しています。`-p` で承認が必要な操作が自動で拒否されたときは、PostToolUse も Stop も届かず、セッションは作業中のまま残り、agy の終了後の見回りで待機へ戻ります。`install` が `~/.gemini/config/hooks.json` に登録したフックで、ターミナルの対話の画面の会話もパネルに表示され、セッションを押すと Ghostty のタブへ移動することを確認しています。中断とエラーは確認していません |
| Antigravity | Gemini のデスクトップアプリ | 一部確認済み | `~/.gemini/config/hooks.json` のフックが、ワークスペースを選択せずに始めた会話で届くことを確認しています。パネルにセッションが表示されて作業中から完了へ移り、アプリが付けたタイトルとコンテキストのトークン数が表示されることと、セッションを押すと Gemini のアプリが前面に表示されることを確認しています |
| Antigravity | Windows | 未確認 | 動作を確認していません |

## リリースを作る

正式なリリースは、`vX.Y.Z` という名前のタグを main のコミットに付けて作成します。最初の正式なリリースは `v0.1.0` です。更新用のスクリプトは、既定でこの正式なリリースのうち最新のものを導入します（[更新する](usage.ja.md#更新する)を参照）。

macOS では、`tools/release.sh` にバージョンを一つ渡すと、以下の手順をまとめて実行します。

```bash
tools/release.sh 0.1.3-beta.2
```

スクリプトは、main にいて未コミットの変更が無いことと、同じ名前のタグがまだ無いことを確認し、main の最新を pull してから、四つのファイルのバージョンと `Cargo.lock` のワークスペースのクレートのバージョンを書き換えます。それを `Bump the version to <バージョン>` というコミットにして main へ push し、main の CI がテストとビルドを終えるまで待ちます。CI が通ったときのみ `v<バージョン>` のタグを push し、タグの CI がリリースの公開を終えるまで待って、リリースのアドレスを表示します。main の CI が失敗したときはタグを付けずに止まるため、修正して main へ push してから、同じバージョンで再実行します。バージョンのファイルがすでにそのバージョンなら、コミットを作成せずに現在の main の先端にタグを付けます。スクリプトには GitHub CLI（`gh`）のログインが必要です。

手動で行うときは、以下のとおりです。

1. 以下の四つのファイルのバージョンを同じ値に変更します。`Cargo.toml` の `[workspace.package]` の `version`、`app/src-tauri/tauri.conf.json` の `version`、`app/package.json` の `version`、`app/package-lock.json` の先頭の `version` と `packages[""]` の `version` です。`app/` で `npm version X.Y.Z --no-git-tag-version` を実行すると、`package.json` と `package-lock.json` の両方が変わります。`Cargo.toml` を変更したあとに `cargo build` を一度実行すると `Cargo.lock` のバージョンも変わるため、一緒にコミットします。
2. 変更を main へコミットして push します。
3. main の CI が終わったら、そのコミットに `vX.Y.Z` のタグを付けて push します。CI が終わる前に付けても公開はされますが、タグの CI が検査とビルドをやり直します。

```bash
git tag v0.1.0
git push origin v0.1.0
```

タグの push で CI が実行され、タグから先頭の `v` を除いたバージョンが四つのファイルのバージョンと一致することと、タグの付いたコミットが main に含まれていること（プレリリースでは、main か `feature/` で始まるブランチに含まれていること）を確認します。どちらかを満たさなければ、ビルドせずに理由を表示して失敗します。そのときは、ローカルのタグを `git tag -d vX.Y.Z` で、GitHub のタグを `git push origin :refs/tags/vX.Y.Z` で削除し、修正したコミットに付け直します。

確認を終えると、そのコミットについて main への push で作成した `build-` のプレリリースを検索します。三つのファイルが揃っていれば、検査とビルドをやり直さず、`SHA256SUMS` と照合したうえでそのファイルをコピーして、`marimo vX.Y.Z` という題のリリースとして公開します。タグの付いたコミットは main の実行で検査とビルドを済ませているため、同じものを作り直さないためです。main の実行がまだ終わっていないときや、古いビルドが削除されていたときは、検査が通ったあとに main のビルドと同じ名前のファイルを macOS と Windows でビルドして公開します。バージョンを上げたコミットを main へ push したら、その CI が終わってからタグを push すると、タグの実行はファイルをコピーするのみで終わります。バージョンに `-` を含まないタグは、正式なリリース（プレリリースではないリリース）として公開し、最新のリリース（Latest）にします。リリースノートは GitHub が前のリリースからの変更をもとに作成します。失敗した実行を再実行したときは、同じリリースのファイルを差し替えます。

`v0.2.0-beta.1` や `v0.2.0-rc.1` のようにバージョンに `-` を含むタグは、SemVer のプレリリースとして扱います。バージョンを確認するときは `-beta.1` のような後ろの部分も含めて四つのファイルと比較するため、ファイルのバージョンも `0.2.0-beta.1` のように記載します。CI はこのタグを GitHub のプレリリースとして公開し、最新のリリース（Latest）にはしません。更新用のスクリプトは既定ではプレリリースを選択せず、設定のウィンドウからの更新はどのバージョンのアプリからでもプレリリースを選択しないため、プレリリースを公開しても利用者の更新先は変わりません。プレリリースを導入するのは、`tools/update.sh --pre`（Windows では `tools\update.ps1 -Pre`）のみです。正式なリリースを公開したときは、`-` より前のバージョンがそれと同じか古い `v` で始まるプレリリースを、タグごと削除します。たとえば `v0.2.0` を公開すると `v0.2.0-beta.1` を削除し、`v0.2.1-beta.1` は残します。これらのプレリリースは、より新しいリリースがあるので、`--pre` で選択されることがもう無いためです。

プレリリースは、マージする前のブランチからも出せます。`feature/` で始まるブランチをチェックアウトして、`tools/release.sh 0.2.0-beta.1` のように `-` を含むバージョンを渡すと、スクリプトはバージョンのコミットをそのブランチに作って push し、続けてそのコミットにタグを付けて push します。ブランチへの push では CI がビルドしないため、タグの CI が検査とビルドを行ってから公開します。`-` を含まない正式なリリースは、main のコミットからのみ作成できます。ブランチを main へマージすると、ブランチで上げたバージョンも main に入ります。

## アイコンを描き直す

アイコンを描くスクリプトは、[素材を作り直す](customize.ja.md#素材を作り直す)の手順で作成した Python の仮想環境で実行します。アイコンはビルド時にアプリへ埋め込まれるため、描き直したらアプリを再ビルドします。

メニューバーと通知領域のアイコンは、`tools/tray_icon.py` が Pillow のみでまりもの形に描きます。実行すると `app/src-tauri/icons/` へ四つの PNG を書き出します。`tray-template.png` は macOS の通常のアイコンで、メニューバーの明るさに合わせて macOS が白か黒で描くテンプレート画像です。Windows の通知領域はこの描き分けをしないため、`tray-color.png` を Windows の通常のアイコンにしています。`tray-waiting.png` と `tray-error.png` は、承認待ちとエラーのときに両方の OS で使用するアイコンです（見え方は[メニューバーと通知領域のアイコン](usage.ja.md#メニューバーと通知領域のアイコン)を参照）。メニューバーは画像の高さを 18 ポイントに縮めて配置するため、いずれも 2 倍の画面で等倍になる 36×36 ピクセルにしています。

アプリのアイコンは、`tools/app_icon.py` が水槽の中の顔のあるまりもを描きます。毛並みは乱数で置いた短い線を数万本重ねて描き、乱数の種を固定しているため、何度実行しても同じ画像になります。実行すると `app/src-tauri/icons/` へ `icon.icns`、`icon.ico`、`icon.png`、`32x32.png`、`128x128.png`、`128x128@2x.png` を書き出します。macOS 用の `icon.icns` は、1024×1024 ピクセルの画像の中央 824 ピクセルに角丸の正方形を描いて周りを透明にし、Dock に並ぶ他のアプリと大きさを揃えます。Windows と Linux 用の残りのファイルは、同じ絵から角丸の正方形のみを切り出して画像いっぱいに描きます。

いずれもリポジトリの直下で実行します。

```bash
.venv/bin/python tools/tray_icon.py
.venv/bin/python tools/app_icon.py
```

## 仕組みの概要

### データの流れ

データは以下の順に流れます。

1. 各ツールから呼ばれた `marimo-hook` が、標準入力の JSON を読み、セッションの新しい状態を決定して状態ファイルに書き込みます。Claude Code 以外のツールの入力は、Claude Code の入力に読み替えてから同じ規則で状態を決定します（[状態の決め方](#状態の決め方)を参照）。呼ばれ方と書き込み先は以下のとおりです。

   | 呼び出し元 | サブコマンド | 書き込み先 |
   | --- | --- | --- |
   | Claude Code のフック | `marimo-hook hook` | `sessions/<session_id>.json` |
   | Codex のフック | `marimo-hook codex-hook` | `sessions/codex-<session_id>.json`。利用制限は `codex_rate_limits.json` |
   | Hermes のフック | `marimo-hook hermes-hook` | `sessions/hermes-<session_id>.json` |
   | OpenCode のサービスの中で動作するプラグイン | `marimo-hook opencode-hook` | `sessions/opencode-<session_id>.json` |
   | agy のフック | `marimo-hook antigravity-hook` | `sessions/antigravity-<conversationId>.json`。transcript から読んだトークン数をコンテキスト使用率として記録 |
   | Claude Code の statusLine | `marimo-hook statusline` | コンテキスト使用率とセッションの名前をセッションのファイルへ。値が変わらない間は書き直さず、同じ値でも 60 秒ごとに書き直す。新しい応答が届いていれば、利用制限を `rate_limits.json` へ |

   OpenCode のプラグインは、イベントを購読し、選択したイベントごとに `marimo-hook` を起動します。前に起動した `marimo-hook` が終了してから次を起動するため、書き込みの順がイベントの順と入れ替わりません。フックはツールの処理を止めてしまうため、ロックを待つのは最大 300 ミリ秒までにしています。
2. アプリは、Claude Code が `~/.claude.json` に残した利用制限を 30 秒ごとに確認して `rate_limits.json` へ書き込みます。再取得の間隔を選択していれば、`codex app-server` に問い合わせた利用制限を `codex_rate_limits.json` へ、`agy -p /usage` で読んだ利用制限を `antigravity_rate_limits.json` へ書き込みます（[marimo が起動するもの](#marimo-が起動するもの)を参照）。
3. アプリはファイルの監視（file watcher。フォルダの中のファイルが変わると OS から通知を受け取る仕組み）で `sessions/` と `rate_limits.json` の変化を受け取ります。並列のツール呼び出しで書き込みが重なるため、120 ミリ秒変化が無くなるまで待ち、遅くとも 500 ミリ秒で、全セッションをまとめたスナップショットを読みます。
4. Rust 側は、スナップショットの全体の状態に合わせてメニューバーと通知領域のアイコンを切り替え、スナップショットを `snapshot` という名前のイベントで画面部分へ送ります。アイコンを画面部分に任せないのは、ウィンドウを隠している間は画面部分が止まることがあるためです。macOS 14 以降では、隠したウィンドウの画面部分を WebKit が止めないよう、`tauri.conf.json` の `backgroundThrottling` を `disabled` にしています。Tauri はこの設定を macOS 13 以前と Windows では扱いません。
5. 画面部分は、既読の記録を加味して全体の状態を決定し直し、表情、吹き出し、パネルを描き直します。アイコンのメニューの先頭に並べるセッションの一覧も画面部分が決定し、変わったときのみ Rust 側へ送ります。メニューでセッションを選択すると、Rust 側はそのセッションのキーを `tray-session` という名前のイベントで画面部分へ返し、画面部分がパネルのセッションを押したときと同じ操作を行います。

### 状態の決め方

Claude Code のセッションは、届いたフックによって以下の状態になります。ほかのツールのイベントの読み替えは、ツールごとの「記録するもの」の節に記載しています。

| 状態 | この状態になるイベント |
| --- | --- |
| 承認待ち | ツールの実行許可の確認（PermissionRequest。サブエージェントからのものも含む）、質問（AskUserQuestion）、計画の承認（ExitPlanMode）、入力の要求（Elicitation）、承認待ちの通知 |
| エラー | StopFailure（利用制限に達したときなど） |
| 作業中 | プロンプトの送信、ツールの実行の前後、サブエージェントの開始（SubagentStart） |
| 完了 | Stop、最後のサブエージェントの終了（SubagentStop） |
| 待機 | セッションの開始、中断や承認拒否のあとに届く待機の通知（idle_prompt） |

完了を作業中より下に置いているのは、あるセッションの作業を見守っている間に、ほかのセッションが終わるたびに立ち絵の表情が切り替わらないようにするためです。完了したことはパネルのセッションの色で分かります。

ツールごとの「作業の要約」では、各ツールのツールを、要約の作り方が同じ Claude Code のツールに置き換えます。置き換え先の無いツールは、ツール名と引数をそのまま短く表示します。

以下は Claude Code のサブエージェントの扱いです。Hermes と OpenCode のサブエージェントの扱いは、[Hermes のフックから記録するもの](#hermes-のフックから記録するもの)と [OpenCode のプラグインから記録するもの](#opencode-のプラグインから記録するもの)に記載しています。

サブエージェント（Claude が Agent ツールで起動する別の Claude）の中で発火したフックも、親のセッションと同じ session_id で届きます。marimo は、フックの入力にある `agent_id` の項目でこれを区別します。hooks のドキュメントの Common input fields によると、この項目はサブエージェントの中で発火したときのみ入ります。`--agent` で起動したセッションでは親の会話のフックにも `agent_type` が入るため、`agent_type` は区別に使用しません。

サブエージェントのフックは、パネルに表示する名前も、セッション自身の進捗も書き換えません。worktree で隔離されたサブエージェントが別のフォルダで動作しても、パネルに表示する名前は親のセッションのリポジトリ名やフォルダ名のままです。marimo は SubagentStart と SubagentStop で実行中のサブエージェントを記録し、一つでも実行中の間は、親のセッションの応答が終わったあとでも作業中として表示します。最後のサブエージェントが終わると完了になり、完了までにかかった時間もそこまでで数えます。サブエージェントがツールの実行許可を求めたときや質問したときは承認待ちとして表示し、許可、拒否、回答が済むと元の表示に戻します。

サブエージェントの記録は、以下のときに外します。

- 前景で動かしたサブエージェントを利用者が途中で止めたときは、SubagentStop が届きません。前景のサブエージェントは親の応答が終わる前に必ず終わるため、親の会話の Stop が届いたときに、その入力の `background_tasks`（バックグラウンドで動作し続けている作業の一覧）に含まれないサブエージェントを外します。一覧の `id` が SubagentStart の `agent_id` と同じ値であることは hooks のドキュメントに記載されていないため、一覧のサブエージェントが記録と一つも一致しないときは、何も外しません。
- 利用者が Esc で親の応答ごと止めたときは Stop も届きません。そこで、待機の通知（idle_prompt）が届いたときに、記録しているサブエージェントをすべて外します。Claude Code はこの通知を、バックグラウンドで動作するサブエージェントが一つも無くなってから送ります。親の会話が完了かエラーのときは、未読の結果なので、状態はそのまま残します。
- サブエージェントが失敗したときなどにも SubagentStop が届くとは限らないため、30 分の間何のイベントも届かないサブエージェントは終了したものとして扱います。

### プロセスが異常な終わり方をしたとき

どのツールも、プロセスが落ちたり強制終了されたりすると、SessionEnd にあたるフックやイベントを送りません。Claude Code が中断のあとに送る待機の通知（idle_prompt）も、プロセスが無くなれば届きません。そのままでは、作業中や承認待ちのセッションが、24 時間後に削除されるまで残ります。

そこで、フックはターンを動かしているプロセスの ID と起動時刻を `host` に記録します。記録するのは、セッションの開始時、プロンプトの送信時、まだ記録の無いセッションへのフックのときです。`--resume` で別のプロセスへ移った会話は、そこで新しいプロセスに置き換わります。フックの親がシェルのとき（sh、bash、zsh、dash、ksh、fish、cmd、powershell、pwsh と、環境変数 `SHELL` と `COMSPEC` が指すもの）は、シェルを飛ばした先の祖先を記録します。シェルはフックと一緒に終了するためです。ツールごとに記録されるプロセスは以下のとおりです。

| ツール | フックの起動のしかた | 記録するプロセス |
| --- | --- | --- |
| Claude Code | macOS では `/bin/sh` が exec でフックに置き換わる。Windows の Git Bash の bash.exe は exec しても親として残る | claude のプロセス |
| Codex | macOS では `SHELL` のシェルの `-lc` で、Windows では `COMSPEC`（無ければ cmd.exe）の `/C` で起動する（codex-rs/hooks/src/engine/command_runner.rs の build_command） | Codex のプロセス |
| Hermes | シェルを経由せずに起動する | Hermes のプロセスそのもの |
| OpenCode | プラグインがシェルを経由せずに起動する | プラグインを動かしている OpenCode のサービスのプロセス |
| Antigravity | Unix では `sh -c` で起動する | ターミナルの `agy` のプロセス |

アプリは起動時と 10 分ごとに、作業中か承認待ちのセッションのプロセスが終了していれば、中断されたものとして待機へ戻し、実行中だったサブエージェントの記録も外します。親の会話が完了かエラーで、サブエージェントが実行中のために作業中と表示されていたセッションは、未読の結果なので完了かエラーのまま残し、サブエージェントの記録のみを外します。

待機のセッションのプロセスが終了していれば、そのセッションのファイルを削除します。Claude Code がバックグラウンドで用意しておく予備のプロセス（`claude bg-spare`）は、自分の session_id で SessionStart を送って待機のセッションを作成し、使われないまま 60 分ほどで終了しても SessionEnd を送らないためです。待機のセッションはパネルに表示されないため、ファイルを削除したセッションがまだ続いていても見え方は変わらず、次のフックでファイルが作成し直されます。

プロセス ID は再利用されるため、起動時刻が記録と異なれば別のプロセスとみなします。macOS では、親に回収される前のゾンビの状態も終了したものとみなします。権限が足りないなどで確認できないときは、動作しているものとみなします。

### バックグラウンドへ移した会話

Claude Code で会話をバックグラウンドへ移すと、会話は新しい session_id のセッションで続き、そのセッションが自分の状態ファイルを作成します。元のセッションには SessionEnd も Stop も届かず、元の CLI のプロセスも動作し続けるため、元のセッションは作業中のまま残ります。

そこで、フックは親の会話のフックの入力にある `transcript_path` を状態ファイルに記録します。アプリは起動時と 10 分ごとに、記録した会話ログの末尾を読み、最後の親の会話の行（`isSidechain` が true でない `user` か `assistant` の行）より後に `continued-in` の行があれば、元のセッションを削除します。`continued-in` は、移したときに Claude Code が元の会話ログに書き込む行です。移したあとでも元のセッションが応答を続けることがあり、そのときは `continued-in` の後に会話の行が続くため削除しません。この行の形は Claude Code のドキュメントに無く、ローカルの会話ログで確認したものです。

### 利用者が止めた応答

Claude Code で利用者が Esc で応答を止めたときや、ツールの実行許可や質問を断ったときは、Stop も PermissionDenied も届きません。CLI はその後 60 秒ほどで待機の通知（idle_prompt）を送るため、そこで待機へ戻します。デスクトップアプリはこの通知を送らず、プロセスも動作し続けるため、作業中や承認待ちのセッションが次のプロンプトか 24 時間後まで残ります。

そこで、アプリは起動時と 10 分ごとに、記録した会話ログの末尾を読みます。最後の親の会話の行が、中断を表す文章（`[Request interrupted by user]` か `[Request interrupted by user for tool use]`）のみの `user` の行なら、作業中か承認待ちのセッションを待機へ戻します。止めたあとに送ったプロンプトがまだ会話ログに書き込まれていないこともあるため、プロンプトを送った時刻がその行の `timestamp` より後なら変更しません。バックグラウンドで動作するサブエージェントは止めたあとも動作し続けるため、その時刻より後にイベントが届いたサブエージェントは残し、それより前から動きの無いもののみを外します。この行の形も Claude Code のドキュメントに無く、ローカルの会話ログで確認したものです。

### タイトルとコンテキスト使用率の取り方

Claude Code のセッションの場合を記載します。ほかのツールは、ツールごとの「記録するもの」の節を参照してください。

デスクトップアプリの Code タブでは statusLine が動作しないため、marimo はコンテキスト使用率を会話ログ（transcript。Claude Code がセッションごとに書き込む JSON Lines 形式の記録）から数えます。このときモデルのコンテキストの上限は分かりません。

タイトルは二つの場所から取得します。CLI では statusLine の入力にある `session_name` を使用します。これは `--name` や `/rename` で付けた名前か、それが無ければ Claude Code が自動で付けたタイトルです。statusLine が呼ばれないデスクトップアプリの Code タブでは、PostToolUse と Stop のときに読む会話ログの末尾からタイトルの行を検索します。この行の形式は Claude Code のドキュメントに記載されていません。どちらの場合も、タイトルが付く前や最初の読み取りの前は名前のみが表示されます。

### Codex のフックから記録するもの

- **状態** セッションの開始、プロンプトの送信、ツールの実行の前後、実行許可の確認（PermissionRequest）、応答の終了（Stop）、サブエージェントの開始と終了を、Claude Code と同じ状態に対応させます。Codex の質問のツール `request_user_input` は、Claude Code の AskUserQuestion と同じく承認待ちにします。Codex がサンドボックス外への書き込みなどの権限を `request_permissions` で求めたときは、PermissionRequest が届かないため、このツールの PreToolUse で承認待ちにし、許可か拒否が済んで PostToolUse が届くと作業中へ戻します。利用者がターンを中断したときに届く Interrupt では、作業中や承認待ちから待機へ戻します。
- **作業の要約** シェルのコマンド（Codex はツール名 `Bash` で送ります）、ファイルの編集（`apply_patch`。パッチに記載されたファイルの名前を表示します）、MCP（Model Context Protocol。外部のツールを AI に接続する仕組み）のツール、サブエージェントの起動（`spawn_agent`）、画像の表示（`view_image`）、質問（`request_user_input`）を要約します。
- **コンテキスト使用率** Codex が会話ごとに書き込む記録（rollout。`~/.codex/sessions/` の下の JSON Lines 形式のファイル）の末尾から最後の `token_count` を読み、Codex の画面の下に表示される残りの割合と同じ式で計算します。Codex は、システムプロンプトなどで常に使用される 12000 トークンをコンテキストの上限と使用量の両方から引いて割合を出すため、marimo も同じように引き、100 からその残りの割合を引いた値を使用率として記録します。読むのはセッションの開始、PostToolUse、Stop のときのみです。
- **タイトル** セッションの開始、プロンプトの送信、Stop のときに、`~/.codex/session_index.jsonl` の末尾からそのセッションの最後の `thread_name` を読みます。Codex のデスクトップアプリは、プロジェクトを選択せずに始めた会話ごとに `~/Documents/Codex/<日付>/<最初のプロンプトから作った名前>` という作業フォルダを作成します。このフォルダ名はタイトルの言い換えにすぎないため、パネルに表示する名前と吹き出しの `{folder}` には、フォルダ名の代わりにタイトルを使用します。
- **プログラムから始めた会話** 状態ファイルを作成するのは、rollout の先頭の `session_meta` にある `originator` が、利用者が自分で開くクライアントの名前のときのみです。対象の名前は、デスクトップアプリの `Codex Desktop`、ターミナルの Codex CLI の `codex-tui`、VS Code の拡張機能の `codex_vscode` の三つです。`codex exec`（`codex_exec`）や、Claude Code から作業を任された codex-async（`codex-async-bridge`）と Claude Code 向けの Codex プラグイン（`Claude Code`）のように、プログラムから始めた会話はパネルに並べません。Claude Code から任された会話なら、任せた側の Claude Code のセッションがすでに作業中を示しているためです。rollout が無いか、先頭の行が 64 KB に収まらず `originator` を読めない会話には、状態ファイルを作成しません。デスクトップアプリが提案を作成するためにバックグラウンドで始める会話（cwd が `/` で、`~/.codex/ambient-suggestions` に結果を書き込むもの）は rollout を残さないため、ここで除外されます。自分で始めた会話でも、rollout が作成される前に届いたフックでは状態ファイルを作成せず、`originator` を読めた次のフックから並べます。SessionEnd は `transcript_path` を持たずに届くため、`originator` を読めて作成済みの状態ファイルは、読めないフックが届いても削除しません。利用制限はアカウント全体の値なので、除外した会話の rollout からも記録します。`originator` の値は Codex の文書に無く、macOS と Windows のローカルの rollout で確認したものです。app-server を経由して始めた会話では、クライアントが名乗った `clientInfo.name` が入ります。
- **利用制限** rollout の同じ `token_count` にある利用制限を、`~/.marimo/codex_rate_limits.json` に書き込みます。再取得の間隔を選択していれば、アプリも `codex app-server` へ問い合わせて同じファイルに書き込むため、Codex を使用していない間も値が古くなりません。Codex の利用制限のウィンドウは、プランによって 5 時間と 7 日の二つだったり 7 日の一つのみだったりするため、ウィンドウの長さを分単位のまま記録します。複数のセッションの rollout はそれぞれ別の時点の値を持つため、記録済みの値より新しい時点の値でのみ書き換えます。

### Hermes のフックから記録するもの

Hermes のフックの入力は、どのイベントでも `hook_event_name`、`tool_name`、`tool_input`、`session_id`、`cwd` を持ち、それ以外の値を `extra` にまとめて持ちます（Hermes の `agent/shell_hooks.py`）。

- **状態** 以下のように読み替えます。Hermes の `on_session_end` は会話の終わりではなく、ターンの終わりごとに届くイベントです。

  | Hermes のイベント | 読み替え先 | 状態 |
  | --- | --- | --- |
  | `on_session_start`、`on_session_reset` | SessionStart | 待機。完了とエラーのセッションはそのまま残す |
  | `pre_llm_call` | UserPromptSubmit | 作業中 |
  | `pre_tool_call`、`post_tool_call` | PreToolUse、PostToolUse | 作業中。聞き返しのツール `clarify` の間のみ承認待ち |
  | `pre_approval_request` | PermissionRequest | 承認待ち（危険なコマンドの承認を利用者に求めている間）。自動の承認（smart approval）が判定役の LLM に聞いている間（`surface` が `smart`）は、利用者を待っていないので無視する |
  | `post_approval_response` | PostToolUse か PermissionDenied | 作業中 |
  | `post_llm_call`、`on_session_end`（`completed`） | Stop | 完了 |
  | `on_session_end`（`interrupted`） | Interrupt | 待機 |
  | `on_session_end`（`failed`）、`api_request_error`（`retryable` が偽） | StopFailure | エラー |
  | `on_session_finalize` | SessionEnd | セッションを削除する。完了とエラーのセッションは残す |

- **裏の見直し** Hermes は応答のあとに、同じ session_id のまま記憶とスキルの見直しをバックグラウンドで実行します。この見直しは `pre_llm_call` を出さずにツールを使用するため、`pre_llm_call` で通知されたターンの ID（`extra.turn_id`）と異なる ID のイベントは捨てます。捨てないと、完了したセッションが作業中へ戻ってしまいます。
- **作業の要約** `terminal` は Bash、`read_file`、`write_file`、`patch` はそれぞれ Read、Write、Edit、`search_files` は Grep、`web_search` と `web_extract` は WebSearch と WebFetch、`delegate_task` は Agent、`clarify` は Codex の質問のツールの `request_user_input` として扱います。
- **コンテキスト使用率** モデルを呼ぶたびに届く `post_api_request` の `usage.prompt_tokens` を、`context_length` で割って求めます。`prompt_tokens` は、キャッシュから読んだ分も含めてモデルに渡した入力のトークン数です。
- **タイトルと開く先** ターンの始まりに、Hermes のフォルダの `sessions/sessions.json` から session_id が一致する行を検索します。このファイルは gateway がチャットごとの現在のセッションを書き込む対応表で、Hermes 自身はこれを `state.db` のミラー（legacy mirror）と呼んでいます。チャンネルでは `サーバー名 / #チャンネル名` の形の名前からチャンネル名のみをタイトルにし、DM では相手の名前を使わずに `DM` をタイトルにします。Discord の会話では、サーバーとチャンネル（DM では DM のチャンネル）の ID から `discord://-/channels/<サーバー>/<チャンネル>` の形の URL を組み立てて記録します。CLI の会話はこの表に含まれないため、タイトルも URL もありません。
- **作業フォルダ** gateway は Hermes のフォルダを作業フォルダにして動作するため、その場所は会話の居場所ではありません。Hermes のフォルダと同じ `cwd` は記録せず、パネルに表示する名前と吹き出しの `{folder}` にはタイトルを使用します。CLI の会話は、起動したフォルダを記録します。
- **プロセスの終わり** cron のジョブのセッションは、終わりを通知するフック（`on_session_finalize`）を出しません。gateway や CLI が異常な終わり方をしたときも同じです。そこで、ほかのツールと同じく、ターンを動かしているプロセスが終了したセッションを待機へ戻します（[プロセスが異常な終わり方をしたとき](#プロセスが異常な終わり方をしたとき)を参照）。gateway を再起動したときに作業中だった cron のジョブのセッションも、これで遅くとも 10 分ほどでパネルから消えます。
- **起動元** Hermes のフックの祖先のプロセスは会話のウィンドウではないため、起動元の手がかりは記録しません。パネルのセッションを押したときは、記録した URL が marimo の組み立てる Discord のチャンネルの形であることを確認してから開きます。Windows の Discord は、動作中の本体へリンクを渡して画面を切り替えるのみで前面には表示されないため、marimo が Discord のウィンドウを前面に表示します。URL の無いセッションでは何もしません。
- **記録しないもの** 利用制限は記録しません。Hermes のフックの入力に、利用制限の値が無いためです。サブエージェント（`delegate_task` で動作する子のエージェント）は自分の session_id でフックを出すため、親とは別のセッションとしてパネルに表示されます。gateway の `/stop` は session_id を持たないイベントのみを出すため、止めたセッションは作業中のまま残ることがあり、次のターンで正しい状態に戻ります。会話の圧縮で Hermes が session_id を付け替えたときは、新しい ID のセッションが表示され、古いセッションは 24 時間で削除されます。

### OpenCode のプラグインから記録するもの

OpenCode v2 のプラグイン（`install` が OpenCode の設定のフォルダの `plugins/marimo.js` に配置するもの）は、以下のイベントを選択して `marimo-hook opencode-hook` に渡します。親を持つセッション（サブエージェント）のイベントは、ターンの始まりと終わり、実行許可と質問のもののみを親のセッションへ渡し、ほかはプラグインが捨てます。

| OpenCode のイベント | 読み替え先 | 状態 |
| --- | --- | --- |
| `session.created` | SessionStart | 待機。タイトルと作業フォルダ（`location.directory`）を記録する |
| `session.renamed` | なし | タイトルのみを書き換える |
| `session.execution.started` | UserPromptSubmit | 作業中 |
| `session.tool.input.started`、`session.tool.called` | PreToolUse | 作業中。ツールの名前と引数を Claude Code のものに置き換える |
| `session.tool.success`、`session.tool.failed` | PostToolUse、PostToolUseFailure | 作業中 |
| `permission.asked` | PermissionRequest | 承認待ち |
| `permission.replied` | PostToolUse か PermissionDenied（`reply` が `reject`） | 作業中 |
| `form.created` | PreToolUse（AskUserQuestion） | 承認待ち（質問） |
| `form.replied`、`form.cancelled` | PostToolUse | 作業中 |
| `session.execution.succeeded` | Stop | 完了。そのターンで最後に書かれた応答の文章を要約に使用する |
| `session.execution.failed` | StopFailure | エラー。`session.step.failed` と `session.retry.scheduled` は再試行されるため使用しない |
| `session.execution.interrupted`（`reason` が `superseded` 以外） | Interrupt | 待機 |
| `session.deleted` | SessionEnd | セッションを削除する。完了とエラーのセッションも削除する |
| `session.step.ended` | なし | コンテキスト使用率のみを書き換える。トークン数はそのステップの入力、出力、推論、キャッシュの合計で、上限はモデルの一覧から取得できたときのみ使用する |

- **対象** プラグインは OpenCode v2 のプラグインの仕組み（`setup` で受け取る `ctx` の `ctx.event.subscribe`）でイベントを購読します。プラグインは、ターミナルの画面（TUI）ではなく、会話を動かすサービスの中で読み込まれます。対象は、ターミナルの画面が接続するバックグラウンドのサービス、`--standalone` で起動した専用のサーバー、`opencode serve` のサーバーです。v1 のデスクトップアプリには対応しません。
- **Claude Code から任された会話** プラグインを読み込んだプロセスの環境変数 `CLAUDECODE` が `1` で、そのプロセスがバックグラウンドのサービス（引数に `--service` を持つ `opencode serve`）でないときは、イベントを購読せず、状態ファイルを作成しません。Claude Code はツールから起動したプロセスの環境に `CLAUDECODE=1` を設定し、`opencode run --standalone` が起動する専用のサーバーはそれを引き継ぎます。任せた側の Claude Code のセッションがすでに作業中を示しているため、同じ作業を別のセッションとして並べないためです。バックグラウンドのサービスは、Claude Code の中から最初に起動されると `CLAUDECODE=1` を持ったまま、利用者がターミナルの画面から開いた会話も動かすため、この区別に使用しません。OpenCode の会話の記録には、Codex の `originator` のように会話を始めたクライアントを表す値が無いため、`--standalone` を付けずにサービスへ頼んだ `opencode run` の会話は区別できず、パネルに表示されます。
- **サブエージェント** 親を持つセッションは、プラグインが作成のイベントで親を記録し、自分の状態ファイルを作成させません。作成のイベントを見ていないセッションは、OpenCode に親を問い合わせてから決定します。`opencode-hook` も、親を持つ `session.created` を無視します。親のセッションは、サブエージェントを動かすツール（`subagent`）を使用している作業中として表示されます。サブエージェントのターンの始まりの `session.execution.started` と、`permission.asked`、`permission.replied`、`form.created`、`form.replied`、`form.cancelled` と、ターンの終わりの `session.execution.succeeded`、`session.execution.failed`、`session.execution.interrupted` のみは、親をたどった先の親を持たないセッションの ID を `parent` に追加して渡し、ほかのイベントは渡しません。`opencode-hook` はこれを、そのセッションの中で動作する Claude Code のサブエージェントのイベントと同じに扱います。ターンの始まりは SubagentStart として、実行中のサブエージェントに数えます。実行許可や質問の回答を待つ間は承認待ちとして表示し、回答が済んだらサブエージェントが実行中の作業中として表示し、サブエージェントのターンが終わったら（`reason` が `superseded` の中断を除きます）親の会話の状態に戻します。バックグラウンドで動かしたサブエージェントが、親の応答が終わったあとに回答を求めたときも同じです。
- **途中から見たセッション** プラグインを読み込む前から続いているセッションのように、作成のイベントを見ていないセッションを初めて見たときは、OpenCode にそのセッションのタイトルと作業フォルダも問い合わせて、イベントに追加して渡します。
- **作業の要約** `shell` は Bash、`read`、`write`、`edit` はそれぞれ Read、Write、Edit、`patch` は書式が同じ Codex の `apply_patch`、`glob` と `grep` は Glob と Grep、`webfetch` と `websearch` は WebFetch と WebSearch、`subagent` は Agent として扱います。v1 の名前の `bash` と `task` も同じように扱います。ツールの実行のイベントはツールの名前を持たないため、プラグインが入力の始まりのイベント（`session.tool.input.started`）で記録した名前を追加して渡します。実行許可の要求はツールの名前ではなく操作の種類（`action`）と対象（`resources`）で届くため、同じ対応でツールに置き換えます。質問のフォームは、各フィールドの説明を問いの文として、選択肢の見出しと一緒に表示します。
- **コンテキスト使用率** ステップの終わりのトークン数は、そのステップの入力、出力、推論、キャッシュの読み書きの合計で、OpenCode のターミナルの画面がコンテキストの使用量として表示する値と同じ数え方です。セッションの累計を通知する `session.usage.updated` は使用しません。上限は、プラグインが読み込み時に OpenCode のモデルの一覧から取得しておき、一覧に無いモデルに切り替えたときのみ取得し直します。それでも取得できなければ上限を渡さず、パネルにはトークン数が表示されます。
- **イベントの順** プラグインは、前に起動した `marimo-hook` が終了してから次を起動します。別々のプロセスが同時に書き込むと書き込みの順が入れ替わり、完了のあとに作業中へ戻ることがあるためです。待つのはプラグインの中のキューのみで、OpenCode の処理は待たせません。同じ ID のイベントは二度渡しません。OpenCode が購読を切ったときは、1 秒おいて購読し直します。プラグインが外されるとき（`opencode run --standalone` のように OpenCode が終了するときを含みます）は、まだ渡していないイベントをその場で渡し切ります。
- **プロセスの終わり** フックの親は OpenCode のサービスのプロセスなので、サービスが落ちたときや強制終了されたときは、ほかのツールと同じく、作業中や承認待ちのセッションを遅くとも 10 分ほどで待機へ戻します（[プロセスが異常な終わり方をしたとき](#プロセスが異常な終わり方をしたとき)を参照）。
- **起動元** 会話を動かすサービスはターミナルとは別のプロセスなので、起動元の手がかりは記録しません。パネルのセッションを押したときは、macOS の Ghostty で、タイトルが `OC | <会話のタイトル>` のターミナルを検索します。これは OpenCode の TUI がターミナルに付けるタイトルで、タイトルが 40 文字（JavaScript の文字列の長さで、UTF-16 の単位で数えます）を超えるときは、37 文字に切って省略の記号を追加した形になります。タイトルで見つからなければ、作業フォルダが一致するターミナルが一つのときにそれを選択します。見つからなければ、Ghostty も前面に表示せずに何もしません。Terminal.app と iTerm2 は tty（ターミナルの装置名）でしかタブを検索できないため扱わず、Windows では何もしません。
- **記録しないもの** 利用制限は記録しません。プラグインが OpenCode のイベントから利用制限を取り出さないためです。

### Antigravity のフックから記録するもの

agy のフック（`install` が `~/.gemini/config/hooks.json` に登録するもの）は、`marimo-hook antigravity-hook <イベント名>` を呼びます。フックの入力の形は、agy に同梱された agy-customizations の skill の `docs/hooks.md` に従います。Gemini のデスクトップアプリが動かす agy の入力は、CLI の 1.5.8 と異なり `hookEventName` と `cwd` を持たないため、イベント名はコマンドの引数から、作業フォルダは `workspacePaths` の先頭から取得します。ワークスペースを選択せずに始めた会話は作業フォルダを持たないため、Hermes の gateway の会話と同じく、タイトルをパネルに表示する名前にします。

| agy のイベント | 読み替え先 | 状態 |
| --- | --- | --- |
| `PreInvocation`（`invocationNum` が 0） | UserPromptSubmit | 作業中。ターンの最初のモデルの呼び出しの前に届く |
| `PreInvocation`（作業中のセッションで、`invocationNum` が 1 以上か、サブエージェントの返事を受けて動き直したときの 0） | PreInvocation | 作業中のまま。最後のフックとその時刻のみを書き換える |
| `PreToolUse` | PreToolUse | 作業中。ツールの名前と引数を Claude Code のものに置き換える |
| `PostToolUse` | PostToolUse | 作業中 |
| `Stop`（`terminationReason` が `NO_TOOL_CALL` などの正常な終わりで、`fullyIdle` が true） | Stop | 完了。transcript の最後の応答の文章を要約に使用する |
| `Stop`（正常な終わりで、`fullyIdle` が false） | PostToolUse | 作業中のまま。サブエージェントの返事を待つ親の会話は、いったん Stop を出してから、返事を受けて動き直す |
| `Stop`（`terminationReason` が `USER_CANCELED`） | Interrupt | 待機 |
| `Stop`（`terminationReason` が `ERROR`、`MAX_INVOCATIONS`、`MAX_FORCED_INVOCATIONS`、`MAX_TOKEN_BUDGET_EXCEEDED`） | StopFailure | エラー。`terminationReason` を小文字にしてセリフの分類に使用し、`error` の文を要約に使用する |

- **ターンの始まり** agy は 2 ターン目以降に `SessionStart` を出さず、ターンの最初の `PreInvocation` で `invocationNum` を 0 から数え直します。サブエージェントの返事を受けて親の会話が動き直すときも 0 から数え直しますが、そのときはセッションが作業中のままなので、ターンの始まりの時刻を書き換えません。`SessionStart` と `PostInvocation` は登録しません。
- **承認待ち** agy には、承認を求めている間に呼ばれるフックがありません。利用者が承認を断ったときも、agy は PostToolUse も Stop も出さずにターンを止めます。そこでアプリが、作業中か承認待ちの Antigravity のセッションについて、会話の DB（agy のデータのフォルダの `conversations/<会話の ID>.db`。SQLite のファイル）の `steps` の表を、1 秒ごとに読み取り専用で読みます。読むのは、最後のモデルの応答（`step_type` が 15）より後の step のうち、承認を待っているものがあればそれ、無ければ最も新しいものです。一つの応答で複数のツールを呼ぶと、承認を待つ step の後ろに順番を待つ step が並びうるためです。DB の場所は、フックが状態ファイルに記録した transcript の場所（`<データのフォルダ>/brain/<会話の ID>/.system_generated/logs/transcript_full.jsonl`）から求めます。

  | 最後の step | セッションの状態 | 読み替え先 | セッションの状態の変化 |
  | --- | --- | --- | --- |
  | WAITING（9） | 作業中 | PermissionRequest | 承認待ち。step のツールの名前と引数で要約を作成する |
  | ERROR（7）、CANCELED（6）、INTERRUPTED（12）になって 2 秒以上 | 承認待ち | Interrupt | 待機 |
  | RUNNING（2）になって 1 秒以上 | この表の読み替えで承認待ちにしたセッション | PostToolUse | 作業中 |
  | サブエージェントの会話の WAITING | サブエージェントが承認待ちでない | PermissionRequest（サブエージェント） | 親のセッションのサブエージェントを承認待ちにする |
  | サブエージェントの会話の DONE（3）か、ERROR などの終わり | サブエージェントが承認待ち | PostToolUse（サブエージェント） | サブエージェントの承認待ちを外す |

  承認しても、PostToolUse はツールが終わるまで届かず、コマンドでは非同期に切り替わる 5 秒後まで届きません。そこで、step が RUNNING に戻って 1 秒経過したら承認されたとみなし、作業中に戻します。断ったときや Esc で閉じたときも step は RUNNING を通りますが、数ミリ秒で ERROR か CANCELED になります。質問のツールで承認待ちにしたセッションは、WAITING になる前に RUNNING のまま数秒経過することがあるため、この規則を適用しません。承認されたツールが失敗したときも step は ERROR になりますが、そのときはすぐに PostToolUse が届きます。このため、ERROR になってから 2 秒待っても承認待ちのままのときのみ、断ったものとみなします。

  step の状態の値は、agy の実行ファイルに埋め込まれた protobuf の定義から読んだものです。ツールの名前と引数（フィールド 4）と状態の遷移の時刻（フィールド 26）は、`metadata` の列の protobuf から読みます。いずれも文書に無く、agy 1.5.9 の CLI で承認、拒否、Esc を起こして確認した形です。agy は会話を作成するときに DB の表を作成し、そのとき別の agy のプロセスが同じ DB を開いていると「database is locked」で SQLite への保存をやめ、会話を `<会話の ID>.pb` へまとめて書き込む形式に切り替えます。この形式の会話は動作中の step を DB に書き込まないため、承認待ちを表示できず、作業中として表示します。アプリが DB を開くのは、agy が WAL で開いている印の `-wal` のファイルがあるときのみにして、この切り替えの引き金にならないようにしています。Gemini のデスクトップアプリの DB も同じ表の形であることは確認していますが、デスクトップアプリで承認を求めたときの動作は確認していません。利用者に質問するツール（`ask_question`）は、Claude Code の AskUserQuestion と同じく、呼ばれた時点で承認待ち（質問）にします。このツールの名前は agy の実行ファイルの中の文字列から拾ったもので、届くことは確認していません。
- **応答を待つ間に止めたターン** モデルの応答を待っている間に利用者が止めると、agy は Stop も PostInvocation も出さず、会話の DB にも止めた step を残しません。ターンの最初の応答の前でも、ツールを使用したあとの応答の前でも同じで、agy 1.5.9 の CLI で Esc を押して確認しました。応答を待つ間も step は増えないため、止めた会話と、モデルが考えている会話は、フックが来ない時間の長さでしか区別できません。そこで、承認待ちを調べるのと同じ 1 秒ごとの見回りで、以下のすべてに当てはまるセッションを Interrupt に読み替え、待機へ戻します。
  - 作業中で、実行中のサブエージェントが無い
  - 最後のフックが UserPromptSubmit か PreInvocation である。止めたターンは、最後のフックがこのどちらかのまま止まる。PreToolUse のあとはツールが動作しており、`fullyIdle` が false の Stop（PostToolUse に読み替えたもの）のあとはバックグラウンドの作業を待っているため数えない
  - 現在のターン（最後の利用者の入力の step から後）に、PENDING（1）、RUNNING（2）、GENERATING（8）、WAITING（9）の step が無い。応答を生成している間は、応答の step が GENERATING になる。agy が DB を開いておらず読めないときは、この条件を調べない
  - 最後のフックから 10 分経過した

  agy 1.5.9 の会話の DB では、step が増えないまま応答を待った時間は最長 46 秒、考え始めてから応答の step が終わるまでは最長 89 秒でした。誤って待機に戻すと動作中の会話がパネルから消えるため、余裕を大きく取り、プロセスが終了した作業中のセッションを戻すまでの時間（遅くとも 10 分ほど）と揃えています。誤って待機に戻しても、次の PreToolUse か PostToolUse で作業中に、Stop で完了に戻ります。Gemini のデスクトップアプリの agy は、止めたあとの会話の DB を開いていません。デスクトップアプリで最初の応答の前に止めた会話が、DB を読めないまま 10 分で待機に戻ることは確認しましたが、ターンの途中で DB を開いたままにしているかは確認していません。
- **Claude Code から任された会話** フックの環境に `CLAUDECODE` があるときは、状態ファイルを作成せず、既存の状態ファイルを削除します。agy は自分の環境をフックへ引き継ぐため、Claude Code の中から起動した agy（Claude Code が作業を任せた agy-async など）のフックには、Claude Code がツールから起動したプロセスに設定する `CLAUDECODE=1` が見えます。任せた側の Claude Code のセッションがすでに作業中を示しているため、同じ作業を別のセッションとして並べないためです。
- **サブエージェント** agy のサブエージェントは、親とは別の会話の ID でフックを出し、入力に親を指す項目はありません。サブエージェントの transcript の先頭の行は、親からの指示を載せた `SYSTEM_MESSAGE` の行で、その本文が `sender=<親の会話の ID>` を含みます。そこで、transcript の先頭の 2 KB にこの形があるときのみ、サブエージェントの会話とみなし、そのイベントを親のセッションの中で動作するサブエージェントとして記録します。サブエージェントの transcript は最初のツールの呼び出しまで作成されないため、transcript を読めず、まだ状態ファイルの無い会話のイベントは捨てます。利用者が始めた会話の transcript は、最初のフックの前に作成されていることを確認しています。
- **作業の要約** `run_command` は Bash で、agy が付けるコマンドの説明（`toolSummary`）を要約に使用します。`read_file` と `view_file` は Read、`write_file`、`write_to_file`、`create_file` は Write、`edit_file`、`replace_file_content`、`multi_replace_file_content` は Edit、`grep_search` は Grep、`search_web` と `read_url_content` は WebSearch と WebFetch、`invoke_subagent` は Agent として扱います。
- **コンテキスト使用率** transcript の最後のモデルの応答の行にある `input_tokens` と `cache_read_tokens` の合計を、トークン数として表示します。agy はモデルのコンテキストの上限をフックにも transcript にも書き込まないため、パーセントは表示しません。
- **タイトル** Gemini のデスクトップアプリは、会話に付けたタイトルを、agy のデータのフォルダ（`artifactDirectoryPath` の二つ上）の `annotations/<会話の ID>.pbtxt` に、protobuf のテキスト形式の `title:"..."` として書き込みます。ターンの始まりと終わりにこのファイルを読み、タイトルにします。この形式は文書に無く、ローカルのファイルで確認したものです。agy の CLI はこのファイルを作成しないため、CLI の会話には、パネルに作業フォルダの名前のみが表示されます。
- **会話の終わり** agy には会話の終わりを通知するフックがありません。フックの祖先のうちシェルでない最も近いプロセス（ターミナルの CLI では `agy` のプロセス）を記録し、ほかのツールと同じく、そのプロセスが終了した待機のセッションはアプリが削除し、作業中のまま残ったセッションは遅くとも 10 分ほどで待機へ戻します（[プロセスが異常な終わり方をしたとき](#プロセスが異常な終わり方をしたとき)を参照）。完了とエラーのセッションは、既読にするまで残り、24 時間で削除されます。
- **起動元** Claude Code と同じく、フックの環境変数と祖先のプロセスからターミナルを記録し、パネルのセッションを押したときにそのターミナルへ移動します。agy は Ghostty のターミナルのタイトルを `agy` にし、会話のタイトルも無いため、Ghostty では記録した tty が一致するターミナルを検索します。Gemini のデスクトップアプリの会話では、起動元に Gemini のアプリ（`com.google.GeminiMacOS`）が記録されるため、セッションを押すと Gemini のアプリを前面に表示します。
- **記録しないもの** 利用制限はフックの入力に無いため、アプリが `agy -p /usage` で別に読みます（[marimo が起動するもの](#marimo-が起動するもの)を参照）。

### 起動元のアプリの見分け方

macOS のフックは、どのアプリから起動されたかを環境変数 `__CFBundleIdentifier` で取得します。Codex のデスクトップアプリから動作するセッションのように、この変数が無いときは、セッションの開始時とプロンプトの送信時に、フックの祖先のプロセスを近い順にたどります。アプリの中にある実行ファイルが見つかれば、それを含む一番外側のアプリの `Info.plist` から bundle id を読んで記録します。Claude Code の CLI を含む claude.app のように、前面に表示できないアプリ（`Info.plist` に `LSBackgroundOnly` を持つもの）は飛ばして、その先の祖先を検索します。それでもアプリが分からない Codex のセッションでは、rollout の先頭の `session_meta` にある `originator` が `Codex Desktop` なら、Codex のデスクトップアプリ（`com.openai.codex`）から始めた会話として記録します。この値は Codex の文書に無く、ローカルの rollout で確認したものです。

デスクトップアプリの Code タブから動作する Claude Code の CLI の環境には、`CLAUDE_CODE_HOST_SESSION_ID` が入っています。値は `local_` で始まるアプリの中の会話の id で、アプリが `~/Library/Application Support/Claude/claude-code-sessions` に置く会話の記録の `sessionId` と同じであることを確認しています。フックはこれを `desktop_session` として記録し、パネルのセッションを押したときに `claude://code/continue?session=` に渡します。この変数も Claude Code の文書に無いため、値が無いか形が異なるとき（`local_` に英数字とハイフンが 64 文字までのもの以外）は、アプリを前面に表示するのみにします。

### クリックの透過

ウィンドウがクリックを下へ通す状態になると、マウスの移動のイベントもウィンドウに届かなくなります。そこで、画面部分がクリックを受け取る領域（パネルと吹き出しの矩形と、立ち絵の画像のアルファ値から作成した横 40 マスの粗い格子。縦のマス数は素材の縦横比で決まり、小春では 40×60）を Rust 側へ送り、Rust 側はカーソルの位置を定期的に読んで、領域の上にあるときのみウィンドウがクリックを受け取るように切り替えます。カーソルを読む周期は、ウィンドウの上にあるとき 40 ミリ秒、外にあるとき 150 ミリ秒です。アイコンからウィンドウを隠している間はカーソルを読まず、500 ミリ秒ごとにウィンドウが再表示されたかのみを確認します。立ち絵の上にマウスがあるかどうかも、同じ判定と一緒に画面部分へ通知します。
