# marimo

marimo は、AI のコーディングエージェントのセッションの状態を画面の隅で知らせるデスクトップウィジェットです。Claude Code、Codex、Hermes Agent、OpenCode の四つに対応しています。

画面の隅に立つキャラクターが、各セッションが作業中なのか、承認を待っているのか、終わったのかを表情と吹き出しで伝えます。横のパネルには、セッションごとのコンテキスト使用率と、Claude Code と Codex のアカウント全体の利用制限が出ます。別の作業をしていても、エージェントのウィンドウを見に行かずに状況が分かるようにすることが marimo の目的です。

<p align="center">
  <img src="docs/images/screenshot-detail.png" width="516" alt="marimo の画面。立ち絵の頭の上に承認待ちを知らせる吹き出しが出て、左のパネルに四つのセッションと利用制限が並んでいる">
</p>

上の画面では、gaussian-scan のセッションがコマンドの実行許可を待っているので、立ち絵が手を振って吹き出しで知らせています。パネルには承認待ち（橙）、作業中（青）、完了（緑）のセッションが 1 行ずつ並び、いちばん下の行がアカウント全体の利用制限です。

> [!NOTE]
> marimo は有志が作った非公式のツールです。Anthropic をはじめ、marimo が対応しているツールのどの開発元とも関係がなく、どの開発元が承認したものでもありません。Claude と Claude Code は Anthropic の商標です。Codex、ChatGPT、Hermes Agent、OpenCode などのそのほかの製品名は、それぞれの会社の商標または登録商標である場合があります。

## 目次

- [marimo が見せるもの](#marimo-が見せるもの)
- [対応しているツール](#対応しているツール)
- [動作環境](#動作環境)
- [インストール](#インストール)
- [不具合の報告と貢献](#不具合の報告と貢献)
- [ライセンス](#ライセンス)

使い方、見た目の変え方、開発の詳しい説明は次の三つの文書にあります。

| 文書 | 書いてあること |
| --- | --- |
| [marimo の使い方](docs/usage.md) | 起動と終了、設定、パネルの見方、セッションへの移動、立ち絵の表情、利用制限、更新、困ったとき、アンインストール |
| [見た目を変える](docs/customize.md) | セリフの変え方、自分のキャラクターの作り方、素材の作り直し方 |
| [marimo の開発](docs/development.md) | ソースからのビルド、`install` が書き換えるもの、ファイルとデータ、プライバシー、テスト、動作確認の状況、仕組み |

## marimo が見せるもの

### 立ち絵、吹き出し、パネルの役割

marimo のウィンドウは、立ち絵、吹き出し、パネルの三つでできています。

<p align="center">
  <img src="docs/images/expressions.jpg" width="640" alt="既定のキャラクターの表情の一覧。待機、作業中の九種類、承認待ち、完了、エラー、待機中の九つの仕草">
</p>

- **立ち絵**は、全体の状態をひと目で伝えます。作業中は使っているツールによって顔が変わり、待機中はときどき仕草を見せます（[立ち絵の表情](docs/usage.md#立ち絵の表情)を参照）。
- **吹き出し**は、承認待ち、完了、エラーになったときに立ち絵の頭の上に出て、どのリポジトリで何が起きたかをひとことで伝えます。セリフは用意された候補から選ぶので、大規模言語モデル（LLM）は呼びません。
- **パネル**は、待機以外のセッションを 1 行ずつ並べます。各行には、状態の色、ツールのアイコン、リポジトリ名とチャットのタイトル、コンテキスト使用率、今の作業の短い要約、最後に動きがあってからの時間が出ます（[パネルの見方](docs/usage.md#パネルの見方)を参照）。

キャラクターは、既定の小春（こはる）と、Claude Code のマスコットの Clawd から選べます（[設定](docs/usage.md#設定)を参照）。Clawd は、Claude Code の起動時に出るロゴと同じ形のドット絵で描いています。

### 状態の種類と優先度

marimo は、どのツールのセッションも次の五つの状態に分けます。表のトリガーは Claude Code のフックのイベント名で書いていますが、ほかのツールのイベントも同じ名前に読み替えるので、どのツールでも同じ規則で状態が決まります。

| 状態 | 意味 | この状態になるトリガー |
| --- | --- | --- |
| 承認待ち | エージェントが利用者の操作を待っています | ツールの実行許可の確認（PermissionRequest。サブエージェントからのものを含みます）、質問（AskUserQuestion）、計画の承認（ExitPlanMode）、入力の要求（Elicitation）、承認待ちの通知 |
| エラー | 応答が途中で失敗しました | StopFailure（利用制限に達したときなど） |
| 作業中 | エージェントが考えているか、ツールを使っているか、サブエージェントが動いています | プロンプトの送信、ツールの実行の前後、サブエージェントの開始（SubagentStart） |
| 完了 | 応答が終わり、動いているサブエージェントもありません | Stop、最後のサブエージェントの終了（SubagentStop） |
| 待機 | 何もしていません | セッションの開始、中断や承認の拒否のあとに届く待機の通知（idle_prompt） |

完了は応答が終わったことを表すだけで、作業が成功したとは限りません。完了とエラーの行は、まだ読んでいない結果として残り、既読にすると折りたためます（[完了とエラーの既読](docs/usage.md#完了とエラーの既読)を参照）。

複数のセッションが同時に動いているとき、立ち絵にはいちばん優先度の高い状態が出ます。優先度は高い順に、承認待ち、エラー、作業中、完了、待機です。完了を作業中より下に置いているのは、あるセッションの作業を見守っている間に、ほかのセッションが終わるたびに表情が切り替わらないようにするためです。完了したことはパネルの行の色で分かります。

## 対応しているツール

状態を受け取って書き留めるのは、どのツールでも同じコマンド `marimo-hook` です。ツールごとに違うサブコマンドで呼ばれます。

| ツール | 状態を受け取る仕組み | 登録のしかた |
| --- | --- | --- |
| Claude Code | フックと statusLine | `marimo-hook install` が `settings.json` に登録します |
| Codex | フック | `install` が `hooks.json` に登録し、利用者が Codex CLI の `/hooks` で信頼します |
| Hermes Agent | シェルフック | 利用者が `config.yaml` に手で書き、Hermes で承認します |
| OpenCode | プラグイン | `install` が設定のフォルダにプラグインを置き、利用者が OpenCode を起動し直します |

フックは、特定のイベントが起きたときにツールが呼び出す外部コマンドです。statusLine は、Claude Code の CLI（端末で動かすコマンドラインインターフェース）が画面下部の一行を表示するために呼ぶ外部コマンドで、marimo はここからコンテキスト使用率と利用制限を受け取ります。

ツールによって、パネルに出せるものと、行を押したときの移動先が違います。

| 項目 | Claude Code | Codex | Hermes Agent | OpenCode |
| --- | --- | --- | --- | --- |
| タイトル | 会話のタイトル | 会話のタイトル | Discord のチャンネル名か `DM` | 会話のタイトル |
| コンテキスト使用率 | CLI ではパーセント、デスクトップアプリの Code タブではトークン数 | 上限が分かればパーセント | 上限が分かればパーセント、分からなければトークン数 | 上限が分かればパーセント、分からなければトークン数 |
| 利用制限 | 5 時間と 7 日 | Codex が送ってきたウィンドウ | 出ません | 出ません |
| 行を押したときの移動先 | 端末のタブ、エディタのウィンドウ、デスクトップアプリの会話 | Claude Code と同じ | Discord のチャンネル | macOS の Ghostty の端末だけ |
| サブエージェント | 動いている間は親の行が作業中のままです | 親の行が作業中のままです | 親とは別の行になります | 親の行の作業として出します |

どのツールのセッションかは、行のアイコンか文字のマークで見分けます（[パネルの見方](docs/usage.md#パネルの見方)を参照）。行を押したときの動きは[セッションへ移動する](docs/usage.md#セッションへ移動する)に、利用制限の出方は[利用制限の表示](docs/usage.md#利用制限の表示)に書いています。

### Claude Code

statusLine は端末の CLI でだけ呼ばれ、デスクトップアプリの Code タブでは呼ばれません。このため Code タブのセッションでは、コンテキスト使用率を会話ログから数えてトークン数で出し、利用制限は CLI も使っているときだけ出ます。クラウドで動くセッションと Cowork では、ローカルの設定のフックが動かないので表示できません。

### Codex

Codex のフックは、利用者が Codex CLI の `/hooks` で信頼するまで動きません。パネルに並べるのは、デスクトップアプリ、端末の CLI、VS Code の拡張機能から自分で始めた会話だけで、Claude Code が作業を任せるためにバックグラウンドで動かした会話や `codex exec` の会話は並べません。ChatGPT のエージェントの Dots がクラウドで動かす会話は、手でインポートした一覧を試験的に並べられます（[Cloud の手動観測](docs/development.md#cloud-の手動観測)を参照）。

### Hermes Agent

Hermes のフックは `install` では登録しないので、[Hermes のフック](docs/development.md#hermes-のフック)の手順で手で登録します。Discord などのチャットにつないだ gateway のセッションでは、行にチャンネル名が出て、行を押すとそのチャンネルが開きます。

### OpenCode

対象は OpenCode v2 を端末から使うときのセッションで、v1 のデスクトップアプリには対応していません。プラグインは OpenCode の設定のフォルダ（既定は `~/.config/opencode`）があるときだけ置き、OpenCode の設定ファイルは書き換えません（[OpenCode のプラグイン](docs/development.md#opencode-のプラグイン)を参照）。行を押して移動できるのは、macOS の Ghostty の中で動いている端末だけです。

## 動作環境

marimo は macOS で動作を確かめており、Windows 11 でも大半の機能を確かめています。Claude Code は 2.1.139 以降が必要です。marimo は、このバージョンで加わった exec form という書き方でフックを登録するためです。OpenCode は v2 が必要です。

| ツール | 使い方 | 確認の状況 | 補足 |
| --- | --- | --- | --- |
| Claude Code | macOS の端末の CLI | 確認済み | すべての機能が使えます |
| Claude Code | macOS のデスクトップアプリの Code タブ | 確認済み | 利用制限は CLI も使っているときだけ出ます |
| Claude Code | VS Code の拡張機能 | 未確認 | statusLine が動くかどうかを確かめていません |
| Claude Code | Windows | 一部確認済み | 導入、フック、statusLine、セッションへの移動を確かめています |
| Claude Code | Linux | 未確認 | 動作を確かめていません |
| Claude Code | クラウドで動くセッション、Cowork | 対象外 | ローカルの設定のフックが動きません |
| Codex | macOS のデスクトップアプリと端末の CLI | 確認済み | `/hooks` で信頼したフックが届くことを確かめています |
| Codex | Windows の CLI | 一部確認済み | 実行ファイルのパスが条件を満たすときだけフックを登録します |
| Hermes Agent | Windows の Discord の gateway | 一部確認済み | 承認待ち、エラー、CLI の会話は確かめていません |
| OpenCode | macOS の端末の v2 | 一部確認済み | 行を押して移動できるのは Ghostty だけです |
| OpenCode | Windows の端末の v2 | 未確認 | 行を押しても移動しません |
| OpenCode | v1 のデスクトップアプリ | 対象外 | プラグインが読み込まれません |

環境ごとに何を確かめたかは、[動作確認の状況](docs/development.md#動作確認の状況)に書いています。

## インストール

インストールは四つの段階に分かれています。1. から 3. は最初に一度だけ行い、新しいバージョンへの更新は 4. のスクリプトで行います。ここからのコマンドは、リポジトリの直下で実行します。

### 1. ビルドする

最初の導入では、リポジトリを clone して自分でビルドします。必要なツールとそのバージョンは[必要なもの](docs/development.md#必要なもの)に書いています。

まず、各ツールから呼ばれるコマンド `marimo-hook` をビルドします。

```bash
cargo build --release -p marimo-hook
```

次に、画面部分の依存を入れて、Tauri でアプリをまとめます。`npm run tauri -- build` は、画面部分のビルドと Rust 側のビルドをまとめて実行します。

```bash
cd app
npm install
npm run tauri -- build
cd ..
```

できあがるものの場所は[ビルドの成果物](docs/development.md#ビルドの成果物)に書いています。

### 2. 各ツールに登録する

`marimo-hook install` は、Claude Code のフックと statusLine を登録し、Codex のフォルダがあれば Codex のフックを、OpenCode の設定のフォルダがあれば OpenCode のプラグインを登録します。Claude Code 自身の設定ファイル `~/.claude/settings.json` を書き換えるので、Claude Code に頼まずに自分の端末で実行してください。

最初に `--dry-run` を付けると、ファイルを書き換えずに、変更後の settings.json と変更点だけを表示します。

```bash
./target/release/marimo-hook install --dry-run
```

内容に問題がなければ、`--dry-run` を外して実行します。`install` は書き換える前に settings.json のバックアップを取り、既存のフックや設定はそのまま残します（[フックと statusLine の登録](docs/development.md#フックと-statusline-の登録)を参照）。

```bash
./target/release/marimo-hook install
```

`install` のあとに、使っているツールごとに次のことを行います。

| ツール | `install` のあとに行うこと |
| --- | --- |
| Claude Code | 行うことはありません。登録より前から開いていたセッションで状態が出ないときは、そのセッションを開き直します |
| Codex | Codex CLI で `/hooks` を開き、marimo のフックを信頼します（[Codex のフック](docs/development.md#codex-のフック)を参照） |
| Hermes Agent | `config.yaml` へ手でフックを加え、Hermes で承認してから gateway を起動し直します（[Hermes のフック](docs/development.md#hermes-のフック)を参照） |
| OpenCode | 動いている OpenCode を、バックグラウンドのサービスも含めて起動し直します。サービスは `opencode service restart` で起動し直せます |

### 3. アプリを置いて起動する

macOS では、`marimo.app` を `/Applications` へコピーして起動します。初めて開くときに macOS が「開けません」と警告した場合は、[ad-hoc 署名について](docs/development.md#ad-hoc-署名について)の手順で開きます。

```bash
cp -R target/release/bundle/macos/marimo.app /Applications/
open /Applications/marimo.app
```

Windows では、`target\release\bundle\nsis\` にできる `marimo_<バージョン>_x64-setup.exe` を実行します。利用者ごとのインストールなので、管理者の権限は要りません。アプリは `%LOCALAPPDATA%\marimo\marimo.exe` に入り、スタートメニューにショートカットができます。

起動すると画面の右下に立ち絵が現れ、macOS ではメニューバーに、Windows では通知領域にアイコンが出ます。ログインのたびに起動したいときは、右クリックメニューの「設定…」で開くウィンドウで「ログイン時に起動」にチェックを入れます（[marimo の使い方](docs/usage.md#起動と終了)を参照）。

### 4. 更新する

更新には、`cp` で上書きせずにリポジトリの更新用のスクリプトを使います。スクリプトは、GitHub Actions の CI（push のたびに自動でテストやビルドを行う仕組み）がビルドした最新の正式なリリースを取ってきて、アプリを入れ替え、起動し直します。設定のウィンドウの「更新」からも同じ更新ができます。

```bash
tools/update.sh
```

Windows では、PowerShell 7 で次のコマンドを実行します。スクリプトの実行が既定で禁じられている環境でも動くよう、このコマンドの間だけ実行ポリシーを緩めています。PowerShell 7 が入っていないときは、`pwsh` を Windows に最初から入っている Windows PowerShell 5.1 の `powershell` に置き換えても動きます。

```powershell
pwsh -ExecutionPolicy Bypass -File .\tools\update.ps1
```

正式なリリース以外を入れるときは、次のオプションを付けます。

| 入れるもの | macOS | Windows |
| --- | --- | --- |
| プレリリース | `--pre` | `-Pre` |
| main の最新のコミットを CI がビルドしたもの | `--main` | `-Main` |
| 同じものを、CI のビルドができあがるまで最大 30 分待ってから | `--main --wait` | `-Main -Wait` |
| ローカルでビルドしたもの | `--build` | `-Build` |

オプションの細かい動きと、失敗したときの扱いは[更新する](docs/usage.md#更新する)に書いています。marimo を取り除くときは、[アンインストール](docs/usage.md#アンインストール)の手順に従ってください。

## 不具合の報告と貢献

不具合の報告や改善の提案は、GitHub の Issue で受け付けています。次のことを添えてもらえると助かります。

- 再現の手順
- OS（macOS か Windows）とそのバージョン
- どのツール（Claude Code、Codex、Hermes Agent、OpenCode）で起きたかと、そのバージョン
- そのツールを端末とデスクトップアプリのどちらで使っているか

状態ファイルや会話ログを添えるときは、作業フォルダ名やコマンドに人に見せたくない情報が入っていないかを先に確かめてください。settings.json の書き換え、OpenCode のプラグインの配置、会話ログの読み取りなど、安全に関わる問題は、公開の Issue ではなく GitHub の Security Advisories から非公開で報告してください。

プルリクエストを送るときは、[テストと検査](docs/development.md#テストと検査)のコマンドがすべて通ることを確かめ、挙動を変える変更にはその挙動を確かめるテストを付けてください。

## ライセンス

コードは、[MIT License](LICENSE-MIT) と [Apache License 2.0](LICENSE-APACHE) のどちらかを、利用する人が選んで使えます。

立ち絵などの画像とセリフは、[クリエイティブ・コモンズ 表示 4.0 国際ライセンス（CC BY 4.0）](https://creativecommons.org/licenses/by/4.0/deed.ja)で公開しています。対象は `assets/`（`assets/character/clawd/` を除きます）、`art/`、`docs/images/`、`app/src-tauri/icons/`、`app/src-tauri/shipped-dialogue/` です。詳しくは [LICENSE-ASSETS](LICENSE-ASSETS) を見てください。

小春の立ち絵は、OpenAI の画像生成モデルで作った画像を元に、`tools/character/build.py` で顔の差分の合成と背景の除去をして仕上げています。`art/` の元の画像には、生成したサービスが埋め込んだメタデータがそのまま残っています。

`assets/character/clawd/` の Clawd のドット絵は、Anthropic の Claude Code のマスコットを非公式に描いたファンアートで、Claude Code の起動時のロゴの形に従って `tools/character/clawd.py` が描いています。このフォルダは CC BY 4.0 の対象に含めません。marimo は Anthropic の意匠や商標についての権利を何も与えず、この絵は Anthropic による承認や提携を意味しません。`tools/character/clawd.py` そのものはコードなので、ほかのコードと同じライセンスで使えます。

メニューバーと通知領域のアイコンは `tools/tray_icon.py` が、アプリのアイコンは `tools/app_icon.py` が描いた、marimo 独自のまりもの絵です。
