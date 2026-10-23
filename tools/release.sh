#!/usr/bin/env bash
# macOS 用です。リリースを作るのは開発に使う Mac からなので、Windows 用は用意していません。
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
使い方: tools/release.sh <バージョン>

バージョンのファイルを <バージョン> に書き換えて main へコミットし、push します。main の CI がテストとビルドを終えたら、
v<バージョン> のタグを push して、タグの CI がリリースを公開し終えるまで待ちます。
0.1.3 のように - を含まないバージョンは正式なリリースに、0.1.3-beta.2 のように - を含むバージョンはプレリリースになります。
EOF
}

fail() {
  echo "エラー: $*" >&2
  exit 1
}

[[ $# -eq 1 ]] || { usage; exit 1; }
[[ "$1" != "-h" && "$1" != "--help" ]] || { usage; exit 0; }
version="$1"
# SemVer のバージョン。タグの CI も同じバージョンを四つのファイルと照らし合わせる。
[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]] ||
  fail "バージョン $version は 0.1.3 や 0.1.3-beta.2 の形ではありません。"
tag="v$version"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

command -v gh >/dev/null || fail "gh が見つかりません。GitHub CLI を入れてから実行してください。"
gh auth status >/dev/null 2>&1 || fail "gh が GitHub にログインしていません。gh auth login を実行してください。"

branch="$(git branch --show-current)"
[[ "$branch" == "main" ]] || fail "今のブランチは「${branch:-（ブランチなし）}」です。main へ切り替えてから実行してください。"
dirty="$(git status --porcelain --untracked-files=no)"
if [[ -n "$dirty" ]]; then
  echo "$dirty" >&2
  fail "上のファイルにコミットしていない変更があります。コミットするか取り消してから実行してください。"
fi

echo "最新の main を pull しています"
git pull --ff-only
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null || [[ -n "$(git ls-remote --tags origin "refs/tags/$tag")" ]]; then
  fail "タグ $tag はすでにあります。別のバージョンを指定してください。"
fi

echo "バージョンを $version に書き換えています"
# [workspace.package] の version だけが行頭に version = を持つ。依存の version はインライン表の中にある。
sed -i '' -E "s/^version = \"[^\"]*\"$/version = \"$version\"/" Cargo.toml
# tauri.conf.json で "version" を持つのは最上位の一つだけである。
sed -i '' -E "s/^  \"version\": \"[^\"]*\",$/  \"version\": \"$version\",/" app/src-tauri/tauri.conf.json
# package.json と package-lock.json の先頭と packages[""] のバージョンをそろえて書き換える。
(cd app && npm version "$version" --no-git-tag-version --allow-same-version >/dev/null)
# ワークスペースのクレートのバージョンだけを書き換え、依存のバージョンは動かさない。
cargo update --workspace --offline --quiet

found="$(
  {
    sed -n -E 's/^version = "([^"]*)"$/\1/p' Cargo.toml
    sed -n -E 's/^  "version": "([^"]*)",$/\1/p' app/src-tauri/tauri.conf.json
    node -p 'const p = require("./app/package.json"), l = require("./app/package-lock.json"); [p.version, l.version, l.packages[""].version].join("\n")'
  } | sort -u
)"
[[ "$found" == "$version" ]] || fail "バージョンのファイルの書き換えが合いません（$(echo "$found" | tr '\n' ' ')）。git checkout -- . で戻してください。"

git add Cargo.toml Cargo.lock app/src-tauri/tauri.conf.json app/package.json app/package-lock.json
# main の CI が落ちて直したあとに同じバージョンでやり直すときは、バージョンのファイルがもう書き換わっているので、
# コミットせずに今の main の先端にタグを付ける。
if git diff --cached --quiet; then
  echo "バージョンはすでに $version なので、コミットせずに今の main を使います"
else
  git commit -q -m "Bump the version to $version"
fi
sha="$(git rev-parse HEAD)"
echo "main へ push しています（${sha:0:12}）"
git push -q origin main

# push してから実行が一覧に出るまで数秒かかる。
run_of() {
  local event_ref="$1" id
  for _ in $(seq 1 30); do
    id="$(gh run list --workflow CI --commit "$sha" --branch "$event_ref" --json databaseId --jq '.[0].databaseId // empty' 2>/dev/null || true)"
    if [[ -n "$id" ]]; then
      echo "$id"
      return 0
    fi
    sleep 4
  done
  return 1
}

main_run="$(run_of main)" || fail "main の CI の実行が見つかりません。https://github.com/$(gh repo view --json nameWithOwner --jq .nameWithOwner)/actions を確かめてください。タグは付けていません。"
echo "main の CI（${main_run}）がテストとビルドを終えるのを待っています"
gh run watch "$main_run" --exit-status --interval 30 >/dev/null ||
  fail "main の CI（${main_run}）が失敗しました。gh run view $main_run --log-failed で理由を確かめ、直してからバージョンを変えずにもう一度実行してください（バージョンのコミットは main に残っています）。タグは付けていません。"

echo "タグ $tag を push しています"
git tag "$tag" "$sha"
git push -q origin "$tag"

tag_run="$(run_of "$tag")" || fail "タグの CI の実行が見つかりません。リリースができているかを gh release view $tag で確かめてください。"
echo "タグの CI（${tag_run}）がリリースを公開するのを待っています"
gh run watch "$tag_run" --exit-status --interval 15 >/dev/null ||
  fail "タグの CI（${tag_run}）が失敗しました。gh run view $tag_run --log-failed で理由を確かめてください。"
gh release view "$tag" --json url,isPrerelease --jq '"\(if .isPrerelease then "プレリリース" else "正式なリリース" end)を公開しました: \(.url)"'
