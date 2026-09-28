#!/usr/bin/env bash
# macOS 用です。Windows では tools/update.ps1 を使います。
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
使い方: tools/update.sh [--no-pull] [--wait] [--build]

既定では、main の最新を取り込み、そのコミットを CI がビルドしたものを GitHub のリリースから取ってきて入れ替えます。

  --no-pull  ブランチが main であることの確認と取り込みを省き、今チェックアウトしているコミットを使います。
             --build を付けなければ、そのコミットのビルドを取ってくるので、main を通ったコミットでしか使えません。
  --wait     CI のビルドがまだ無いときに、30 秒おきに最大 30 分待ちます。
  --build    取ってくる代わりに手元でビルドします。ネットワークにつながらないときや、main 以外のブランチを試すときに使います。
EOF
}

fail() {
  echo "エラー: $*" >&2
  exit 1
}

pull=1
wait=0
build=0
for arg in "$@"; do
  case "$arg" in
    --no-pull) pull=0 ;;
    --wait) wait=1 ;;
    --build) build=1 ;;
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
    fail "今のブランチは「${branch:-（ブランチなし）}」です。main へ切り替えてから実行してください。ほかのブランチを試すときは --no-pull と --build を付けます。"
  fi
  dirty="$(git status --porcelain --untracked-files=no)"
  if [[ -n "$dirty" ]]; then
    echo "$dirty" >&2
    fail "上のファイルにコミットしていない変更があります。コミットするか、git checkout -- <ファイル> で取り消してから実行してください。"
  fi
  echo "最新の main を取り込んでいます"
  git pull --ff-only
elif [[ $build -eq 1 ]]; then
  echo "--no-pull が指定されたので、今のチェックアウト（$(git branch --show-current || true)）をそのままビルドします"
else
  echo "--no-pull が指定されたので、今のチェックアウト（$(git branch --show-current || true)）のコミットのビルドを取ってきます"
fi

# フォークから使ったときに、フォーク自身のリリースを取りに行けるよう、origin が GitHub ならそこから決める。
repo_slug() {
  local url re
  url="$(git remote get-url origin 2>/dev/null || true)"
  re='^(https://([^@/]+@)?github\.com/|ssh://git@github\.com/|git@github\.com:)([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)$'
  url="${url%/}"
  url="${url%.git}"
  if [[ "$url" =~ $re ]]; then
    echo "${BASH_REMATCH[3]}"
  else
    echo "unknowns53/marimo"
  fi
}

fetch() {
  local url="$1" dest="$2" code
  code="$(curl -sSL --retry 3 --connect-timeout 20 -o "$dest" -w '%{http_code}' "$url")" ||
    fail "$url を取得できませんでした。ネットワークにつながっているかを確かめてください。つながらないときは --build で手元でビルドできます。"
  case "$code" in
    200) return 0 ;;
    404) return 1 ;;
    *) fail "$url を取得できませんでした（HTTP $code）。" ;;
  esac
}

download_build() {
  local slug sha tag base archive not_ready started deadline expected actual
  # CI がビルドするのは arm64 だけで、Intel の Mac では動かない。
  [[ "$(uname -m)" == "arm64" ]] || fail "CI のビルドは arm64 の Mac 用だけです。この Mac では --build を付けて手元でビルドしてください。"
  slug="$(repo_slug)"
  sha="$(git rev-parse HEAD)"
  tag="build-${sha:0:12}"
  base="https://github.com/$slug/releases/download/$tag"
  archive="marimo-macos-arm64.tar.gz"
  not_ready="コミット ${sha:0:12} のビルドが https://github.com/$slug/releases/tag/$tag にありません。CI がまだこのコミットのビルドを終えていないか、ビルドに失敗しています。https://github.com/$slug/actions で進み具合を確かめてください。アプリは差し替えていません。"

  work="$(mktemp -d "${TMPDIR:-/tmp}/marimo-update.XXXXXX")"
  trap 'rm -rf "$work"' EXIT

  echo "$tag のビルドを取ってきています"
  started=$SECONDS
  # リリースを公開した直後の数十秒は、タグが見えていてもダウンロードが 404 を返すことがあるので、--wait が無くても 1 分は確かめ直す。
  if [[ $wait -eq 1 ]]; then
    deadline=$((SECONDS + 1800)) interval=30
  else
    deadline=$((SECONDS + 60)) interval=10
  fi
  while :; do
    if fetch "$base/SHA256SUMS" "$work/SHA256SUMS" && fetch "$base/$archive" "$work/$archive"; then
      break
    fi
    if [[ $SECONDS -ge $deadline ]]; then
      if [[ $wait -eq 1 ]]; then
        fail "30 分待ってもできあがりませんでした。$not_ready"
      fi
      fail "${not_ready}--wait を付けると、できあがるまで最大 30 分待ちます。"
    fi
    if [[ $wait -eq 1 ]]; then
      echo "CI のビルドを待っています（$(((SECONDS - started) / 60)) 分経過、最大 30 分）"
    else
      echo "ビルドがまだ取れないので、$interval 秒後に確かめ直します（最大 1 分）"
    fi
    sleep "$interval"
  done

  expected="$(awk -v f="$archive" '$2 == f || $2 == "*" f { print $1 }' "$work/SHA256SUMS")"
  actual="$(shasum -a 256 "$work/$archive" | awk '{ print $1 }')"
  [[ -n "$expected" ]] || fail "SHA256SUMS に $archive の行がありません。"
  [[ "$expected" == "$actual" ]] || fail "$archive のチェックサムが SHA256SUMS と合いません。アプリは差し替えていません。"

  mkdir "$work/extract"
  tar -xzf "$work/$archive" -C "$work/extract"
  # curl で取ったものには quarantine が付かないが、付いていると初回の起動で Gatekeeper に止められるので、念のため外す。
  xattr -dr com.apple.quarantine "$work/extract" 2>/dev/null || true

  bundle="$work/extract/marimo.app"
  hook="$work/extract/marimo-hook"
  [[ -d "$bundle" ]] || fail "取ってきたものの中にアプリが見つかりません: $archive"
  [[ -x "$hook" ]] || fail "取ってきたものの中に marimo-hook が見つかりません: $archive"
  # 一時フォルダは終わると消えるので、install をやり直すときはスクリプトごと実行し直してもらう。
  retry_install="tools/update.sh をもう一度実行してください"
}

if [[ $build -eq 1 ]]; then
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
  retry_install="$hook install を実行し直してください"
else
  download_build
fi

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
  fail "marimo-hook install が失敗しました。アプリはすでに新しいものに差し替えて起動しています。上のメッセージを確かめてから、$retry_install。"
fi
echo "更新が終わりました"
