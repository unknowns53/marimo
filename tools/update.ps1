# Windows 用です。macOS では tools/update.sh を使います。
# Windows PowerShell 5.1 は BOM の無いスクリプトを ANSI のコード ページで読んで日本語を壊すので、
# このファイルは BOM 付きの UTF-8 で保存する。

<#
.SYNOPSIS
Updates marimo to a newer version.
marimo を新しいバージョンへ更新します。

.DESCRIPTION
By default, downloads the latest stable release from GitHub and reinstalls the app. The repository is not touched.
既定では、最新の正式なリリースを GitHub から取ってきて入れ直します。リポジトリには触れません。

.PARAMETER Pre
Downloads the most recently published release whose name starts with v, including prereleases.
プレリリースも含めて、最も新しく公開された v で始まるリリースを取ってきます。

.PARAMETER Main
Pulls the latest main and downloads the CI build of that commit from the GitHub releases.
main の最新を pull し、そのコミットを CI がビルドしたものを GitHub のリリースから取ってきます。

.PARAMETER NoPull
Skips checking that the branch is main and pulling, and uses the commit currently checked out. Without -Build, it downloads the build of that commit, so it works only for commits that went through main.
ブランチが main であることの確認と pull を省き、今チェックアウトしているコミットを使います。-Build を付けなければ、そのコミットのビルドを取ってくるので、main を通ったコミットでしか使えません。

.PARAMETER Wait
Use together with -Main or -NoPull. When the CI build is not ready yet, checks every 30 seconds for up to 30 minutes.
-Main か -NoPull と一緒に使います。CI のビルドがまだ無いときに、30 秒おきに最大 30 分待ちます。

.PARAMETER Build
Builds locally instead of downloading. Without -NoPull, pulls the latest main before building. Use it when there is no network connection, or to try a branch other than main.
取ってくる代わりにローカルでビルドします。-NoPull が無ければ、main の最新を pull してからビルドします。ネットワークにつながらないときや、main 以外のブランチを試すときに使います。
#>
[CmdletBinding()]
param(
    [switch]$Pre,
    [switch]$Main,
    [switch]$NoPull,
    [switch]$Wait,
    [switch]$Build
)

$ErrorActionPreference = 'Stop'

# marimo_core::Lang と同じ規則で決める。display.json の language、OS の表示言語、英語の順に見る。
function Get-MarimoLang {
    $marimoHome = if ($env:MARIMO_HOME) { $env:MARIMO_HOME } else { Join-Path $HOME '.marimo' }
    try {
        $setting = (Get-Content -LiteralPath (Join-Path $marimoHome 'display.json') -Raw -Encoding UTF8 -ErrorAction Stop | ConvertFrom-Json).language
        if ($setting -ceq 'ja' -or $setting -ceq 'en') { return $setting }
    } catch {
        # 無いときや読めないときは OS の言語に従う。
    }
    if (((Get-UICulture).Name -split '[-_.]')[0] -eq 'ja') { 'ja' } else { 'en' }
}
$lang = Get-MarimoLang

# 選んだ文言を出力に返すので、Write-Host や Fail の引数か代入の右辺で使う。
function Msg([string]$Ja, [string]$En) {
    if ($lang -eq 'ja') { $Ja } else { $En }
}

function Fail([string]$Message) {
    Write-Host "$(Msg 'エラー' 'Error'): $Message" -ForegroundColor Red
    exit 1
}

if ($Main -and $NoPull) { Fail (Msg '-Main と -NoPull は一緒に使えません。' '-Main and -NoPull cannot be used together.') }
if ($Pre -and ($Main -or $NoPull -or $Build)) {
    Fail (Msg '-Pre はリリースを選ぶためのものなので、-Main、-NoPull、-Build とは一緒に使えません。' '-Pre selects a release, so it cannot be used with -Main, -NoPull or -Build.')
}
if ($Wait -and -not ($Main -or $NoPull)) {
    Fail (Msg '-Wait は CI がコミットごとに作るビルドを待つためのものなので、-Main か -NoPull と一緒に使ってください。正式なリリースは公開された時点でできあがっています。' '-Wait waits for the build CI makes for each commit, so use it with -Main or -NoPull. Stable releases are complete as soon as they are published.')
}

# Windows PowerShell 5.1 は、stderr がリダイレクトされていると外部コマンドの stderr の行を
# エラーとして扱い、Stop のもとでは止まってしまう。cargo や npm は進み具合を stderr に書くので、
# 外部コマンドの間だけ Continue にして、成否は終了コードで判断する。
function Invoke-Native([string]$File, [string[]]$Arguments) {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $File @Arguments
    } finally {
        $ErrorActionPreference = $saved
    }
    if ($LASTEXITCODE -ne 0) {
        Fail (Msg "$File $($Arguments -join ' ') が失敗しました（終了コード $LASTEXITCODE）。" "$File $($Arguments -join ' ') failed (exit code $LASTEXITCODE).")
    }
}

function Get-InstalledMarimo([string]$Dir) {
    $prefix = $Dir.TrimEnd('\') + '\'
    @(Get-Process -Name marimo -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and $_.Path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
    })
}

$invokedFrom = (Get-Location).ProviderPath
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).ProviderPath

if ($env:CARGO_TARGET_DIR) {
    # cargo は相対パスを実行した場所から解決するので、app\ で動く Tauri とずれないよう絶対パスにそろえる。
    if (-not [IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
        $env:CARGO_TARGET_DIR = [IO.Path]::GetFullPath((Join-Path $invokedFrom $env:CARGO_TARGET_DIR))
    }
    $target = $env:CARGO_TARGET_DIR
} else {
    $target = Join-Path $root 'target'
}

# フォークから使ったときに、フォーク自身のリリースを取りに行けるよう、origin が GitHub ならそこから決める。
function Get-RepoSlug {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $url = git remote get-url origin 2>$null
    } finally {
        $ErrorActionPreference = $saved
    }
    if ($url -and ("$url".Trim() -match '^(?:https://(?:[^@/]+@)?github\.com/|ssh://git@github\.com/|git@github\.com:)([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+?)(?:\.git)?/?$')) {
        return $Matches[1]
    }
    'unknowns53/marimo'
}

function Save-Url([string]$Url, [string]$Path) {
    # 回線が不安定だと、応答を受け取る前に接続が切れることがあるので、3 回までやり直す。
    for ($attempt = 1; ; $attempt++) {
        try {
            Invoke-WebRequest -Uri $Url -OutFile $Path -UseBasicParsing
            return 200
        } catch {
            $response = $_.Exception.Response
            if ($response) { return [int]$response.StatusCode }
            if ($attempt -gt 3) {
                Fail (Msg "$Url を取得できませんでした。ネットワークにつながっているかを確認してください。つながらないときは -Build でローカルでビルドできます。（$($_.Exception.Message)）" "Could not download $Url. Check your network connection. If you are offline, you can build locally with -Build. ($($_.Exception.Message))")
            }
            Write-Host (Msg "取得できなかったので、5 秒後にやり直します（$attempt / 3 回目、$($_.Exception.Message)）" "Download failed; retrying in 5 seconds (attempt $attempt of 3, $($_.Exception.Message))")
            Start-Sleep -Seconds 5
        }
    }
}

# $Base のリリースから SHA256SUMS とアーカイブを取ってきて確かめ、展開したフォルダを返す。
# 取れないときは $NotReady を表示して止まり、-Wait が無ければ $WaitHint を続けて表示する。
function Save-Build([string]$Work, [string]$Base, [string]$NotReady, [string]$WaitHint) {
    $archive = 'marimo-windows-x64.zip'
    $sumsPath = Join-Path $Work 'SHA256SUMS'
    $archivePath = Join-Path $Work $archive

    $started = Get-Date
    # リリースを公開した直後の数十秒は、タグが見えていてもダウンロードが 404 を返すことがあるので、-Wait が無くても 1 分は確かめ直す。
    if ($Wait) { $deadline = $started.AddMinutes(30); $interval = 30 }
    else { $deadline = $started.AddMinutes(1); $interval = 10 }
    while ($true) {
        $code = Save-Url "$Base/SHA256SUMS" $sumsPath
        if ($code -eq 200) { $code = Save-Url "$Base/$archive" $archivePath }
        if ($code -eq 200) { break }
        if ($code -ne 404) { Fail (Msg "$Base から取得できませんでした（HTTP $code）。" "Could not download from $Base (HTTP $code).") }
        if ((Get-Date) -ge $deadline) {
            if ($Wait) { Fail (Msg "30 分待ってもできあがりませんでした。$NotReady" "The build was not ready after 30 minutes. $NotReady") }
            Fail "$NotReady$WaitHint"
        }
        if ($Wait) { Write-Host (Msg "CI のビルドを待っています（$([int]((Get-Date) - $started).TotalMinutes) 分経過、最大 30 分）" "Waiting for the CI build ($([int]((Get-Date) - $started).TotalMinutes) min elapsed, up to 30 min)") }
        else { Write-Host (Msg "ビルドがまだ公開されていないので、$interval 秒後にもう一度確認します（最大 1 分）" "The build is not published yet; checking again in $interval seconds (up to 1 min)") }
        Start-Sleep -Seconds $interval
    }

    $expected = $null
    foreach ($line in Get-Content -LiteralPath $sumsPath) {
        $parts = $line.Trim() -split '\s+', 2
        if ($parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $archive) { $expected = $parts[0].ToLowerInvariant() }
    }
    if (-not $expected) { Fail (Msg "SHA256SUMS に $archive の行がありません。" "SHA256SUMS has no line for $archive.") }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $archivePath).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { Fail (Msg "$archive のチェックサムが SHA256SUMS と合いません。アプリは入れ直していません。" "The checksum of $archive does not match SHA256SUMS. The app was not reinstalled.") }

    $extract = Join-Path $Work 'extract'
    Expand-Archive -LiteralPath $archivePath -DestinationPath $extract
    # インターネットから来たことを示す Mark of the Web（Zone.Identifier）が付いていると、Windows が実行の前に確認を出して /S の実行が止まるので、念のため外す。
    Get-ChildItem -LiteralPath $extract -File | Unblock-File
    $extract
}

function Get-CommitBuild([string]$Work) {
    $slug = Get-RepoSlug
    # CI はアプリに入らないファイルだけを変えたコミットをビルドしないので、アプリに入るファイルを変えた最後のコミットのビルドを探す。
    # この一覧は .github/workflows/ci.yml の target ジョブと同じにしておく。
    $sha = git log --first-parent -1 --format=%H HEAD -- . ':(exclude)*.md' ':(exclude)docs' ':(exclude)LICENSE-*' ':(exclude)tools' ':(exclude)art' ':(exclude).gitignore'
    if ($LASTEXITCODE -ne 0 -or -not $sha) { Fail (Msg 'git でビルドを探すコミットを調べられませんでした。' 'Could not determine which commit to find a build for (git log failed).') }
    $short = $sha.Substring(0, 12)
    $tag = "build-$short"
    $notReady = Msg "コミット $short のビルドが https://github.com/$slug/releases/tag/$tag にありません。CI がまだこのコミットのビルドを終えていないか、ビルドに失敗しています。https://github.com/$slug/actions で進み具合を確認してください。アプリは入れ直していません。" "There is no build of commit $short at https://github.com/$slug/releases/tag/$tag. CI has not finished building this commit yet, or the build failed. Check its progress at https://github.com/$slug/actions. The app was not reinstalled."

    Write-Host (Msg "$tag のビルドを取ってきています" "Downloading build $tag")
    # 英語では文を空白でつなぐので、続けて表示する文の頭に空白を入れておく。
    Save-Build $Work "https://github.com/$slug/releases/download/$tag" $notReady (Msg '-Wait を付けると、できあがるまで最大 30 分待ちます。' ' Add -Wait to wait up to 30 minutes for it to finish.')
}

function Get-ReleaseBuild([string]$Work) {
    $slug = Get-RepoSlug
    $jsonPath = Join-Path $Work 'release.json'
    if ($Pre) {
        $api = "https://api.github.com/repos/$slug/releases?per_page=50"
        $code = Save-Url $api $jsonPath
        if ($code -ne 200) { Fail (Msg "$api を取得できませんでした（HTTP $code）。" "Could not download $api (HTTP $code).") }
        # 一覧は新しく作られた順に返るので、v で始まる下書きでない最初のものを選ぶ。CI が main のコミットごとに
        # 作る build- のプレリリースは選ばない。
        $release = @(Get-Content -LiteralPath $jsonPath -Raw -Encoding UTF8 | ConvertFrom-Json) |
            ForEach-Object { $_ } |
            Where-Object { -not $_.draft -and $_.tag_name -like 'v*' } |
            Select-Object -First 1
        if (-not $release) {
            Fail (Msg "https://github.com/$slug/releases に v で始まるリリースがまだありません。アプリは入れ直していません。" "There is no release whose tag starts with v at https://github.com/$slug/releases yet. The app was not reinstalled.")
        }
        $tag = $release.tag_name
        $kind = Msg 'プレリリースを含む最新のリリース' 'the latest release including prereleases'
    } else {
        # この API は下書きとプレリリースを除いた最新のリリースを返すので、プレリリースと、CI が main のコミットごとに作る build- のプレリリースは選ばれない。
        $api = "https://api.github.com/repos/$slug/releases/latest"
        $code = Save-Url $api $jsonPath
        if ($code -eq 404) {
            Fail (Msg "https://github.com/$slug/releases に正式なリリースがまだありません。main の最新のビルドを入れるときは -Main を付けてください。アプリは入れ直していません。" "There is no stable release at https://github.com/$slug/releases yet. To install the latest build of main, add -Main. The app was not reinstalled.")
        }
        if ($code -ne 200) { Fail (Msg "$api を取得できませんでした（HTTP $code）。" "Could not download $api (HTTP $code).") }
        $tag = (Get-Content -LiteralPath $jsonPath -Raw -Encoding UTF8 | ConvertFrom-Json).tag_name
        $kind = Msg '正式なリリース' 'the latest stable release'
    }
    if (-not $tag) { Fail (Msg "$api の応答からリリースのタグを読み取れませんでした。" "Could not read the release tag from the response of $api.") }
    $notReady = Msg "リリース $tag に Windows 用のファイルがそろっていません。https://github.com/$slug/releases/tag/$tag を確認してください。アプリは入れ直していません。" "Release $tag is missing the Windows files. Check https://github.com/$slug/releases/tag/$tag. The app was not reinstalled."

    Write-Host (Msg "$kind $tag を取ってきています" "Downloading $tag, $kind")
    Save-Build $Work "https://github.com/$slug/releases/download/$tag" $notReady ''
}

$installDir = Join-Path $env:LOCALAPPDATA 'marimo'
$installedExe = Join-Path $installDir 'marimo.exe'
# 呼び出し元のセッションにある同じ名前の変数を、終了時のクリーンアップで消してしまわないよう空にしておく。
$work = $null

Push-Location -LiteralPath $root
try {
    if (-not $NoPull -and ($Main -or $Build)) {
        $branch = git branch --show-current
        if ($LASTEXITCODE -ne 0) { Fail (Msg 'git でブランチを調べられませんでした。' 'Could not check the branch with git.') }
        if ($branch -ne 'main') {
            if (-not $branch) { $branch = Msg '（ブランチなし）' '(no branch)' }
            Fail (Msg "今のブランチは「$branch」です。main へ切り替えてから実行してください。ほかのブランチを試すときは -NoPull と -Build を付けます。" "The current branch is '$branch'. Switch to main and run again. To try another branch, add -NoPull and -Build.")
        }
        $dirty = git status --porcelain --untracked-files=no
        if ($LASTEXITCODE -ne 0) { Fail (Msg 'git で変更の有無を調べられませんでした。' 'Could not check for changes with git.') }
        if ($dirty) {
            $dirty | ForEach-Object { Write-Host $_ }
            Fail (Msg '上のファイルにコミットしていない変更があります。コミットするか、git checkout -- <ファイル> で取り消してから実行してください。' 'The files above have uncommitted changes. Commit them or discard them with git checkout -- <file>, then run again.')
        }
        Write-Host (Msg '最新の main を pull しています' 'Pulling the latest main')
        Invoke-Native 'git' @('pull', '--ff-only')
    } elseif ($Build) {
        $branch = git branch --show-current
        Write-Host (Msg "-NoPull が指定されたので、今のチェックアウト（$branch）をそのままビルドします" "-NoPull was given, so building the current checkout ($branch) as is")
    } elseif ($NoPull) {
        $branch = git branch --show-current
        Write-Host (Msg "-NoPull が指定されたので、今のチェックアウト（$branch）のコミットのビルドを取ってきます" "-NoPull was given, so downloading the build of the current checkout ($branch)")
    }

    if ($Build) {
        Write-Host (Msg 'marimo-hook をビルドしています' 'Building marimo-hook')
        Invoke-Native 'cargo' @('build', '--release', '-p', 'marimo-hook')

        # npm.ps1 を経由すると PowerShell が引数の -- を取り除くことがあるので、npm.cmd を直接呼ぶ。
        # npm install は npm のバージョンや OS によって package-lock.json を書き直し、次の実行を変更ありで止めてしまう。
        Push-Location -LiteralPath (Join-Path $root 'app')
        try {
            Write-Host (Msg 'アプリの依存をインストールしています' 'Installing the app dependencies')
            Invoke-Native 'npm.cmd' @('ci', '--no-audit', '--no-fund')

            Write-Host (Msg 'アプリをビルドしています' 'Building the app')
            Invoke-Native 'npm.cmd' @('run', 'tauri', '--', 'build', '--bundles', 'nsis')
        } finally {
            Pop-Location
        }

        $hook = Join-Path $target 'release\marimo-hook.exe'
        if (-not (Test-Path -LiteralPath $hook)) { Fail (Msg "ビルドした marimo-hook が見つかりません: $hook" "The built marimo-hook was not found: $hook") }
        $nsisDir = Join-Path $target 'release\bundle\nsis'
        $installer = Get-ChildItem -LiteralPath $nsisDir -Filter 'marimo_*_x64-setup.exe' -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending |
            Select-Object -First 1
        if (-not $installer) { Fail (Msg "ビルドしたインストーラが見つかりません: $nsisDir" "The built installer was not found: $nsisDir") }
        $retryInstall = Msg "$hook install を実行し直してください" "run $hook install again"
    } else {
        # Windows PowerShell 5.1 は TLS 1.2 を既定で使わないことがあり、GitHub への接続を拒まれる。
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        # Windows PowerShell 5.1 の Invoke-WebRequest は進み具合の表示を描くたびに遅くなり、ダウンロードが何倍も長くかかる。
        $ProgressPreference = 'SilentlyContinue'

        $work = Join-Path ([IO.Path]::GetTempPath()) ('marimo-update-' + [Guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $work | Out-Null
        if ($Main -or $NoPull) { $extract = Get-CommitBuild $work }
        else { $extract = Get-ReleaseBuild $work }
        $hook = Join-Path $extract 'marimo-hook.exe'
        if (-not (Test-Path -LiteralPath $hook)) { Fail (Msg "ダウンロードしたファイルの中に marimo-hook.exe が見つかりません。" "marimo-hook.exe was not found in the download.") }
        $installer = @(Get-ChildItem -LiteralPath $extract -Filter 'marimo_*_x64-setup.exe') | Select-Object -First 1
        if (-not $installer) { Fail (Msg "ダウンロードしたファイルの中にインストーラが見つかりません。" "The installer was not found in the download.") }
        # 一時フォルダは終わると消えるので、install をやり直すときはスクリプトごと実行し直してもらう。
        $retryInstall = Msg 'tools\update.ps1 をもう一度実行してください' 'run tools\update.ps1 again'
    }

    $running = @(Get-InstalledMarimo $installDir)
    if ($running.Count -gt 0) {
        Write-Host (Msg '起動中の marimo を終了しています' 'Quitting the running marimo')
        foreach ($p in $running) {
            if ($p.HasExited) { continue }
            try {
                $p.Kill()
            } catch {
                Write-Host (Msg "marimo（PID $($p.Id)）を終了できませんでした: $($_.Exception.Message)" "Could not quit marimo (PID $($p.Id)): $($_.Exception.Message)")
            }
        }
        # 終了させたプロセスは、一覧を取り直すと、終了処理が済む前のプロセスがまだ見えることがあるので、同じオブジェクトの終了を待つ。
        # それでも残るときは、動いているアプリを自分で閉じるインストーラに任せて先へ進む。
        $left = @($running | Where-Object { -not $_.WaitForExit(10000) })
        if ($left.Count -gt 0) {
            Write-Host (Msg 'marimo がまだ終了していないので、インストーラに終了を任せます' 'marimo has not quit yet; leaving it to the installer to close it')
        }
    }

    Write-Host (Msg "$($installer.Name) でアプリを入れ直しています" "Reinstalling the app with $($installer.Name)")
    $setup = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -Wait -PassThru
    if ($setup.ExitCode -ne 0) {
        Fail (Msg "インストーラが失敗しました（終了コード $($setup.ExitCode)）。marimo を終了できなかった場合に起こるので、立ち絵を右クリックして「終了」を選んでから、もう一度実行してください。" "The installer failed (exit code $($setup.ExitCode)). This happens when marimo could not be closed. Right-click the character, choose ""Quit"", then run again.")
    }
    if (-not (Test-Path -LiteralPath $installedExe)) {
        Fail (Msg "インストーラは成功を返しましたが、$installedExe が見つかりません。" "The installer reported success, but $installedExe was not found.")
    }

    Write-Host (Msg 'marimo-hook install を実行しています' 'Running marimo-hook install')
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $hook install
    } finally {
        $ErrorActionPreference = $saved
    }
    $installOk = ($LASTEXITCODE -eq 0)

    Write-Host (Msg 'marimo を起動しています' 'Starting marimo')
    Start-Process -FilePath $installedExe

    if (-not $installOk) {
        Fail (Msg "marimo-hook install が失敗しました。アプリはすでに新しいものに入れ直して起動しています。上のメッセージを確認してから、$retryInstall。" "marimo-hook install failed. The new version of the app has already been installed and started. Check the messages above, then $retryInstall.")
    }
    Write-Host (Msg '更新が終わりました' 'Update complete')
} finally {
    Pop-Location
    if ($work) { Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue }
}
