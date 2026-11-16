#!/usr/bin/env bash
# macOS 用です。Windows では tools/update.ps1 を使います。
set -euo pipefail

# marimo_core::Lang と同じ規則で決める。display.json の language、OS の言語、英語の順に見る。
# OS の言語は sys-locale と同じく優先する言語の先頭を読み、LANG は見ない。ターミナルとアプリで言語が食い違わないようにするため。
detect_lang() {
  local file setting tag
  file="${MARIMO_HOME:-${HOME:-}/.marimo}/display.json"
  # jq の無い環境があるので sed で読む。
  setting="$(sed -n 's/.*"language"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$file" 2>/dev/null | head -n 1 || true)"
  case "$setting" in
    ja | en) echo "$setting"; return ;;
  esac
  # 出力は ( "ja-JP", en ) の形の一覧で、記号を含まない要素は引用符なしで出る。
  tag="$(defaults read -g AppleLanguages 2>/dev/null | sed -n '2p' | tr -d ' \t",' || true)"
  case "${tag%%[-_.]*}" in
    [Jj][Aa]) echo ja ;;
    *) echo en ;;
  esac
}
lang="$(detect_lang)"

msg() {
  if [[ $lang == ja ]]; then
    printf '%s\n' "$1"
  else
    printf '%s\n' "$2"
  fi
}

usage() {
  if [[ $lang == en ]]; then
    cat >&2 <<'EOF'
Usage: tools/update.sh [--pre | --main | --no-pull] [--wait] [--build]

By default, downloads the latest stable release from GitHub and replaces the app. The repository is not touched.

  --pre      Download the most recently published release whose tag starts with v, including prereleases.

  --main     Pull the latest main and download the build that CI made for that commit from GitHub releases.
  --no-pull  Skip checking that the branch is main and skip the pull; use the commit currently checked out.
             Without --build, this downloads the build of that commit, so it only works for commits that went through main.
  --wait     Use with --main or --no-pull. If the CI build is not ready yet, check every 30 seconds for up to 30 minutes.
  --build    Build locally instead of downloading. Without --no-pull, pull the latest main before building.
             Use this when offline or to try a branch other than main.
EOF
    return
  fi
  cat >&2 <<'EOF'
使い方: tools/update.sh [--pre | --main | --no-pull] [--wait] [--build]

既定では、最新の正式なリリースを GitHub から取ってきて入れ替えます。リポジトリには触れません。

  --pre      プレリリースも含めて、最も新しく公開された v で始まるリリースを取ってきます。

  --main     main の最新を pull し、そのコミットを CI がビルドしたものを GitHub のリリースから取ってきます。
  --no-pull  ブランチが main であることの確認と pull を省き、今チェックアウトしているコミットを使います。
             --build を付けなければ、そのコミットのビルドを取ってくるので、main を通ったコミットでしか使えません。
  --wait     --main か --no-pull と一緒に使います。CI のビルドがまだ無いときに、30 秒おきに最大 30 分待ちます。
  --build    取ってくる代わりにローカルでビルドします。--no-pull が無ければ、main の最新を pull してからビルドします。
             ネットワークにつながらないときや、main 以外のブランチを試すときに使います。
EOF
}

fail() {
  echo "$(msg エラー Error): $*" >&2
  exit 1
}

pre=0
main=0
no_pull=0
wait=0
build=0
for arg in "$@"; do
  case "$arg" in
    --pre) pre=1 ;;
    --main) main=1 ;;
    --no-pull) no_pull=1 ;;
    --wait) wait=1 ;;
    --build) build=1 ;;
    -h | --help) usage; exit 0 ;;
    *) usage; fail "$(msg "知らない引数です: $arg" "Unknown argument: $arg")" ;;
  esac
done

if [[ $main -eq 1 && $no_pull -eq 1 ]]; then
  usage
  fail "$(msg "--main と --no-pull は一緒に使えません。" "--main and --no-pull cannot be used together.")"
fi
if [[ $pre -eq 1 && ($main -eq 1 || $no_pull -eq 1 || $build -eq 1) ]]; then
  usage
  fail "$(msg "--pre はリリースを選ぶためのものなので、--main、--no-pull、--build とは一緒に使えません。" "--pre selects a release, so it cannot be used with --main, --no-pull or --build.")"
fi
if [[ $wait -eq 1 && $main -eq 0 && $no_pull -eq 0 ]]; then
  usage
  fail "$(msg "--wait は CI がコミットごとに作るビルドを待つためのものなので、--main か --no-pull と一緒に使ってください。正式なリリースは公開された時点でできあがっています。" "--wait waits for the build CI makes for each commit, so use it with --main or --no-pull. Stable releases are complete as soon as they are published.")"
fi

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

if [[ $no_pull -eq 0 && ($main -eq 1 || $build -eq 1) ]]; then
  branch="$(git branch --show-current)"
  if [[ "$branch" != "main" ]]; then
    fail "$(msg "今のブランチは「${branch:-（ブランチなし）}」です。main へ切り替えてから実行してください。ほかのブランチを試すときは --no-pull と --build を付けます。" "The current branch is '${branch:-(no branch)}'. Switch to main and run again. To try another branch, add --no-pull and --build.")"
  fi
  dirty="$(git status --porcelain --untracked-files=no)"
  if [[ -n "$dirty" ]]; then
    echo "$dirty" >&2
    fail "$(msg "上のファイルにコミットしていない変更があります。コミットするか、git checkout -- <ファイル> で取り消してから実行してください。" "The files above have uncommitted changes. Commit them or discard them with git checkout -- <file>, then run again.")"
  fi
  msg "最新の main を pull しています" "Pulling the latest main"
  git pull --ff-only
elif [[ $no_pull -eq 1 && $build -eq 1 ]]; then
  branch="$(git branch --show-current || true)"
  msg "--no-pull が指定されたので、今のチェックアウト（$branch）をそのままビルドします" "--no-pull was given, so building the current checkout ($branch) as is"
elif [[ $no_pull -eq 1 ]]; then
  branch="$(git branch --show-current || true)"
  msg "--no-pull が指定されたので、今のチェックアウト（$branch）のコミットのビルドを取ってきます" "--no-pull was given, so downloading the build of the current checkout ($branch)"
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
    fail "$(msg "$url を取得できませんでした。ネットワークにつながっているかを確認してください。つながらないときは --build でローカルでビルドできます。" "Could not download $url. Check your network connection. If you are offline, you can build locally with --build.")"
  case "$code" in
    200) return 0 ;;
    404) return 1 ;;
    *) fail "$(msg "${url} を取得できませんでした（HTTP ${code}）。" "Could not download ${url} (HTTP ${code}).")" ;;
  esac
}

prepare_download() {
  # CI がビルドするのは arm64 だけで、Intel の Mac では動かない。
  [[ "$(uname -m)" == "arm64" ]] || fail "$(msg "CI のビルドは arm64 の Mac 用だけです。この Mac では --build を付けてローカルでビルドしてください。" "CI builds are for Apple silicon (arm64) Macs only. On this Mac, add --build to build locally.")"
  work="$(mktemp -d "${TMPDIR:-/tmp}/marimo-update.XXXXXX")"
  trap 'rm -rf "$work"' EXIT
}

# $1 のリリースから SHA256SUMS とアーカイブを取ってきて確かめ、展開したものを bundle と hook に入れる。
# 取れないときは $2 を表示して止まり、--wait が無ければ $3 を続けて表示する。
download_from() {
  local base="$1" not_ready="$2" wait_hint="$3" archive started deadline interval expected actual
  archive="marimo-macos-arm64.tar.gz"
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
        fail "$(msg "30 分待ってもできあがりませんでした。$not_ready" "The build was not ready after 30 minutes. $not_ready")"
      fi
      fail "${not_ready}${wait_hint}"
    fi
    if [[ $wait -eq 1 ]]; then
      msg "CI のビルドを待っています（$(((SECONDS - started) / 60)) 分経過、最大 30 分）" "Waiting for the CI build ($(((SECONDS - started) / 60)) min elapsed, up to 30 min)"
    else
      msg "ビルドがまだ公開されていないので、$interval 秒後にもう一度確認します（最大 1 分）" "The build is not published yet; checking again in $interval seconds (up to 1 min)"
    fi
    sleep "$interval"
  done

  expected="$(awk -v f="$archive" '$2 == f || $2 == "*" f { print $1 }' "$work/SHA256SUMS")"
  actual="$(shasum -a 256 "$work/$archive" | awk '{ print $1 }')"
  [[ -n "$expected" ]] || fail "$(msg "SHA256SUMS に $archive の行がありません。" "SHA256SUMS has no line for $archive.")"
  [[ "$expected" == "$actual" ]] || fail "$(msg "$archive のチェックサムが SHA256SUMS と合いません。アプリは差し替えていません。" "The checksum of $archive does not match SHA256SUMS. The app was not replaced.")"

  mkdir "$work/extract"
  tar -xzf "$work/$archive" -C "$work/extract"
  # curl で取ったものには quarantine が付かないが、付いていると初回の起動で Gatekeeper に止められるので、念のため外す。
  xattr -dr com.apple.quarantine "$work/extract" 2>/dev/null || true

  bundle="$work/extract/marimo.app"
  hook="$work/extract/marimo-hook"
  [[ -d "$bundle" ]] || fail "$(msg "ダウンロードしたファイルの中にアプリが見つかりません: $archive" "The app was not found in the download: $archive")"
  [[ -x "$hook" ]] || fail "$(msg "ダウンロードしたファイルの中に marimo-hook が見つかりません: $archive" "marimo-hook was not found in the download: $archive")"
  # 一時フォルダは終わると消えるので、install をやり直すときはスクリプトごと実行し直してもらう。
  retry_install="$(msg "tools/update.sh をもう一度実行してください" "run tools/update.sh again")"
}

download_commit_build() {
  local slug sha tag
  prepare_download
  slug="$(repo_slug)"
  # CI はアプリに入らないファイルだけを変えたコミットをビルドしないので、アプリに入るファイルを変えた最後のコミットのビルドを探す。
  # この一覧は .github/workflows/ci.yml の target ジョブと同じにしておく。
  sha="$(git log --first-parent -1 --format=%H HEAD -- . ':(exclude)*.md' ':(exclude)docs' ':(exclude)LICENSE-*' ':(exclude)tools' ':(exclude)art' ':(exclude).gitignore')"
  [[ -n "$sha" ]] || fail "$(msg "git でビルドを探すコミットを調べられませんでした。" "Could not determine which commit to find a build for (git log failed).")"
  tag="build-${sha:0:12}"
  msg "$tag のビルドを取ってきています" "Downloading build $tag"
  # 英語では文を空白でつなぐので、続けて表示する文の頭に空白を入れておく。
  download_from "https://github.com/$slug/releases/download/$tag" \
    "$(msg "コミット ${sha:0:12} のビルドが https://github.com/$slug/releases/tag/$tag にありません。CI がまだこのコミットのビルドを終えていないか、ビルドに失敗しています。https://github.com/$slug/actions で進み具合を確認してください。アプリは差し替えていません。" "There is no build of commit ${sha:0:12} at https://github.com/$slug/releases/tag/$tag. CI has not finished building this commit yet, or the build failed. Check its progress at https://github.com/$slug/actions. The app was not replaced.")" \
    "$(msg "--wait を付けると、できあがるまで最大 30 分待ちます。" " Add --wait to wait up to 30 minutes for it to finish.")"
}

# 一覧は新しく作られた順に返るので、v で始まる下書きでない最初のものを選ぶ。CI が main のコミットごとに
# 作る build- のプレリリースは選ばない。リリースの項目は 4 文字の字下げで、中の assets の項目と区別できる。
newest_v_release() {
  awk '
    /^    "tag_name": "/ { tag = $0; sub(/^    "tag_name": "/, "", tag); sub(/".*/, "", tag) }
    /^    "draft": false/ && tag ~ /^v/ { print tag; exit }
  ' "$1"
}

download_release() {
  local slug api tag kind
  prepare_download
  slug="$(repo_slug)"
  if [[ $pre -eq 1 ]]; then
    api="https://api.github.com/repos/$slug/releases?per_page=50"
    fetch "$api" "$work/release.json" || fail "$(msg "$api を取得できませんでした（HTTP 404）。" "Could not download $api (HTTP 404).")"
    tag="$(newest_v_release "$work/release.json")"
    [[ -n "$tag" ]] || fail "$(msg "https://github.com/$slug/releases に v で始まるリリースがまだありません。アプリは差し替えていません。" "There is no release whose tag starts with v at https://github.com/$slug/releases yet. The app was not replaced.")"
    kind="$(msg "プレリリースを含む最新のリリース" "the latest release including prereleases")"
  else
    # この API は下書きとプレリリースを除いた最新のリリースを返すので、プレリリースと、CI が main のコミットごとに作る build- のプレリリースは選ばれない。
    api="https://api.github.com/repos/$slug/releases/latest"
    fetch "$api" "$work/release.json" ||
      fail "$(msg "https://github.com/$slug/releases に正式なリリースがまだありません。main の最新のビルドを入れるときは --main を付けてください。アプリは差し替えていません。" "There is no stable release at https://github.com/$slug/releases yet. To install the latest build of main, add --main. The app was not replaced.")"
    tag="$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$work/release.json" | head -n 1)"
    kind="$(msg "正式なリリース" "the latest stable release")"
  fi
  [[ -n "$tag" ]] || fail "$(msg "$api の応答からリリースのタグを読み取れませんでした。" "Could not read the release tag from the response of $api.")"
  msg "${kind} $tag を取ってきています" "Downloading $tag, ${kind}"
  download_from "https://github.com/$slug/releases/download/$tag" \
    "$(msg "リリース $tag に macOS 用のファイルがそろっていません。https://github.com/$slug/releases/tag/$tag を確認してください。アプリは差し替えていません。" "Release $tag is missing the macOS files. Check https://github.com/$slug/releases/tag/$tag. The app was not replaced.")" \
    ""
}

if [[ $build -eq 1 ]]; then
  msg "marimo-hook をビルドしています" "Building marimo-hook"
  cargo build --release -p marimo-hook

  msg "アプリの依存をインストールしています" "Installing the app dependencies"
  # npm install は npm のバージョンや OS によって package-lock.json を書き直し、次の実行を変更ありで止めてしまう。
  (cd app && npm ci --no-audit --no-fund)

  msg "アプリをビルドしています" "Building the app"
  (cd app && npm run tauri -- build --bundles app)

  bundle="$target/release/bundle/macos/marimo.app"
  hook="$target/release/marimo-hook"
  [[ -d "$bundle" ]] || fail "$(msg "ビルドしたアプリが見つかりません: $bundle" "The built app was not found: $bundle")"
  [[ -x "$hook" ]] || fail "$(msg "ビルドした marimo-hook が見つかりません: $hook" "The built marimo-hook was not found: $hook")"
  retry_install="$(msg "$hook install を実行し直してください" "run $hook install again")"
elif [[ $main -eq 1 || $no_pull -eq 1 ]]; then
  download_commit_build
else
  download_release
fi

if pgrep -f "$running_pattern" >/dev/null; then
  msg "起動中の marimo を終了しています" "Quitting the running marimo"
  osascript -e 'tell application id "com.marimo.desktop" to quit' >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    pgrep -f "$running_pattern" >/dev/null || break
    sleep 0.5
  done
  if pgrep -f "$running_pattern" >/dev/null; then
    fail "$(msg "marimo が終了しません。右クリックメニューの「終了」で終了してから、もう一度実行してください。アプリは差し替えていません。" "marimo did not quit. Choose Quit from the right-click menu, then run again. The app was not replaced.")"
  fi
fi

msg "$app_dest を差し替えています" "Replacing $app_dest"
rm -rf "$app_new" "$app_old"
ditto "$bundle" "$app_new"
if [[ -e "$app_dest" ]]; then
  mv "$app_dest" "$app_old"
fi
if ! mv "$app_new" "$app_dest"; then
  [[ -e "$app_old" ]] && mv "$app_old" "$app_dest"
  fail "$(msg "新しいアプリを $app_dest へ置けませんでした。元のアプリに戻しています。" "Could not move the new app to $app_dest. The original app has been restored.")"
fi
rm -rf "$app_old"

msg "marimo-hook install を実行しています" "Running marimo-hook install"
install_ok=1
"$hook" install || install_ok=0

msg "marimo を起動しています" "Starting marimo"
open "$app_dest"

if [[ $install_ok -eq 0 ]]; then
  fail "$(msg "marimo-hook install が失敗しました。アプリはすでに新しいものに差し替えて起動しています。上のメッセージを確認してから、${retry_install}。" "marimo-hook install failed. The app has already been replaced with the new version and started. Check the messages above, then ${retry_install}.")"
fi
msg "更新が終わりました" "Update complete"
