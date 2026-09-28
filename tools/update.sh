#!/usr/bin/env bash
# macOS 用です。Windows では tools/update.ps1 を使います。
set -euo pipefail

usage() {
  echo "使い方: tools/update.sh [--no-pull]" >&2
}

fail() {
  echo "エラー: $*" >&2
  exit 1
}

pull=1
for arg in "$@"; do
  case "$arg" in
    --no-pull) pull=0 ;;
    -h | --help) usage; exit 0 ;;
    *) usage; fail "知らない引数です: $arg" ;;
  esac
done

invoked_from="$PWD"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  # cargo は相対パスを実行した場所から解決するので、app/ で動く Tauri とずれないよう絶対パスにそろえる。
  case "$CARGO_TARGET_DIR" in
    /*) ;;
    *) CARGO_TARGET_DIR="$invoked_from/$CARGO_TARGET_DIR" ;;
  esac
  export CARGO_TARGET_DIR
  target="$CARGO_TARGET_DIR"
else
  target="$root/target"
fi

app_dest="/Applications/marimo.app"
app_new="/Applications/.marimo.app.new"
app_old="/Applications/.marimo.app.old"
running_pattern="$app_dest/Contents/MacOS/"

if [[ $pull -eq 1 ]]; then
  branch="$(git branch --show-current)"
  if [[ "$branch" != "main" ]]; then
    fail "今のブランチは「${branch:-（ブランチなし）}」です。main へ切り替えてから実行してください。ほかのブランチを試すときは --no-pull を付けます。"
  fi
  dirty="$(git status --porcelain --untracked-files=no)"
  if [[ -n "$dirty" ]]; then
    echo "$dirty" >&2
    fail "上のファイルにコミットしていない変更があります。コミットするか、git checkout -- <ファイル> で取り消してから実行してください。"
  fi
  echo "最新の main を取り込んでいます"
  git pull --ff-only
else
  echo "--no-pull が指定されたので、今のチェックアウト（$(git branch --show-current || true)）をそのままビルドします"
fi

echo "marimo-hook をビルドしています"
cargo build --release -p marimo-hook

echo "アプリの依存をインストールしています"
# npm install は npm の版や OS によって package-lock.json を書き直し、次の実行を変更ありで止めてしまう。
(cd app && npm ci --no-audit --no-fund)

echo "アプリをビルドしています"
(cd app && npm run tauri -- build --bundles app)

bundle="$target/release/bundle/macos/marimo.app"
hook="$target/release/marimo-hook"
[[ -d "$bundle" ]] || fail "ビルドしたアプリが見つかりません: $bundle"
[[ -x "$hook" ]] || fail "ビルドした marimo-hook が見つかりません: $hook"

if pgrep -f "$running_pattern" >/dev/null; then
  echo "起動中の marimo を終了しています"
  osascript -e 'tell application id "com.marimo.desktop" to quit' >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    pgrep -f "$running_pattern" >/dev/null || break
    sleep 0.5
  done
  if pgrep -f "$running_pattern" >/dev/null; then
    fail "marimo が終了しません。右クリックメニューの「終了」で終了してから、もう一度実行してください。アプリは差し替えていません。"
  fi
fi

echo "$app_dest を差し替えています"
rm -rf "$app_new" "$app_old"
ditto "$bundle" "$app_new"
if [[ -e "$app_dest" ]]; then
  mv "$app_dest" "$app_old"
fi
if ! mv "$app_new" "$app_dest"; then
  [[ -e "$app_old" ]] && mv "$app_old" "$app_dest"
  fail "新しいアプリを $app_dest へ置けませんでした。元のアプリに戻しています。"
fi
rm -rf "$app_old"
echo "ad-hoc 署名が変わったので、利用制限を API から取得している場合はキーチェーンの確認がもう一度出ることがあります"

echo "marimo-hook install を実行しています"
install_ok=1
"$hook" install || install_ok=0

echo "marimo を起動しています"
open "$app_dest"

if [[ $install_ok -eq 0 ]]; then
  fail "marimo-hook install が失敗しました。アプリはすでに新しいものに差し替えて起動しています。上のメッセージを確かめてから、$hook install を実行し直してください。"
fi
echo "更新が終わりました"
