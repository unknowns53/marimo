# Windows 用です。macOS では tools/update.sh を使います。
# Windows PowerShell 5.1 は BOM の無いスクリプトを ANSI のコード ページで読んで日本語を壊すので、
# このファイルは BOM 付きの UTF-8 で保存する。

<#
.SYNOPSIS
marimo を新しい版へ更新します。

.DESCRIPTION
既定では、main の最新を取り込み、そのコミットを CI がビルドしたものを GitHub のリリースから取ってきて入れ直します。

.PARAMETER NoPull
ブランチが main であることの確認と取り込みを省き、今チェックアウトしているコミットを使います。-Build を付けなければ、そのコミットのビルドを取ってくるので、main を通ったコミットでしか使えません。

.PARAMETER Wait
CI のビルドがまだ無いときに、30 秒おきに最大 30 分待ちます。

.PARAMETER Build
取ってくる代わりに手元でビルドします。ネットワークにつながらないときや、main 以外のブランチを試すときに使います。
#>
[CmdletBinding()]
param(
    [switch]$NoPull,
    [switch]$Wait,
    [switch]$Build
)

$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    Write-Host "エラー: $Message" -ForegroundColor Red
    exit 1
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
        Fail "$File $($Arguments -join ' ') が失敗しました（終了コード $LASTEXITCODE）。"
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
    try {
        Invoke-WebRequest -Uri $Url -OutFile $Path -UseBasicParsing
        return 200
    } catch {
        $response = $_.Exception.Response
        if ($response) { return [int]$response.StatusCode }
        Fail "$Url を取得できませんでした。ネットワークにつながっているかを確かめてください。つながらないときは -Build で手元でビルドできます。（$($_.Exception.Message)）"
    }
}

function Get-Build([string]$Work) {
    # Windows PowerShell 5.1 は TLS 1.2 を既定で使わないことがあり、GitHub への接続を拒まれる。
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    # Windows PowerShell 5.1 の Invoke-WebRequest は進み具合の表示を描くたびに遅くなり、ダウンロードが何倍も長くかかる。
    $ProgressPreference = 'SilentlyContinue'

    $slug = Get-RepoSlug
    $sha = git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { Fail 'git で今のコミットを調べられませんでした。' }
    $short = $sha.Substring(0, 12)
    $tag = "build-$short"
    $base = "https://github.com/$slug/releases/download/$tag"
    $archive = 'marimo-windows-x64.zip'
    $sumsPath = Join-Path $Work 'SHA256SUMS'
    $archivePath = Join-Path $Work $archive
    $notReady = "コミット $short のビルドが https://github.com/$slug/releases/tag/$tag にありません。CI がまだこのコミットのビルドを終えていないか、ビルドに失敗しています。https://github.com/$slug/actions で進み具合を確かめてください。アプリは入れ直していません。"

    Write-Host "$tag のビルドを取ってきています"
    $started = Get-Date
    $deadline = $started.AddMinutes(30)
    while ($true) {
        $code = Save-Url "$base/SHA256SUMS" $sumsPath
        if ($code -eq 200) { $code = Save-Url "$base/$archive" $archivePath }
        if ($code -eq 200) { break }
        if ($code -ne 404) { Fail "$base から取得できませんでした（HTTP $code）。" }
        if (-not $Wait) { Fail "${notReady}-Wait を付けると、できあがるまで最大 30 分待ちます。" }
        if ((Get-Date) -ge $deadline) { Fail "30 分待ってもできあがりませんでした。$notReady" }
        Write-Host "CI のビルドを待っています（$([int]((Get-Date) - $started).TotalMinutes) 分経過、最大 30 分）"
        Start-Sleep -Seconds 30
    }

    $expected = $null
    foreach ($line in Get-Content -LiteralPath $sumsPath) {
        $parts = $line.Trim() -split '\s+', 2
        if ($parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $archive) { $expected = $parts[0].ToLowerInvariant() }
    }
    if (-not $expected) { Fail "SHA256SUMS に $archive の行がありません。" }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $archivePath).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { Fail "$archive のチェックサムが SHA256SUMS と合いません。アプリは入れ直していません。" }

    $extract = Join-Path $Work 'extract'
    Expand-Archive -LiteralPath $archivePath -DestinationPath $extract
    # インターネットから来た印（Zone.Identifier）が付いていると、Windows が実行の前に確認を出して /S の実行が止まるので、念のため外す。
    Get-ChildItem -LiteralPath $extract -File | Unblock-File
    $extract
}

$installDir = Join-Path $env:LOCALAPPDATA 'marimo'
$installedExe = Join-Path $installDir 'marimo.exe'
# 呼び出し元のセッションにある同じ名前の変数を、最後の片付けで消してしまわないよう空にしておく。
$work = $null

Push-Location -LiteralPath $root
try {
    if (-not $NoPull) {
        $branch = git branch --show-current
        if ($LASTEXITCODE -ne 0) { Fail 'git でブランチを調べられませんでした。' }
        if ($branch -ne 'main') {
            if (-not $branch) { $branch = '（ブランチなし）' }
            Fail "今のブランチは「$branch」です。main へ切り替えてから実行してください。ほかのブランチを試すときは -NoPull と -Build を付けます。"
        }
        $dirty = git status --porcelain --untracked-files=no
        if ($LASTEXITCODE -ne 0) { Fail 'git で変更の有無を調べられませんでした。' }
        if ($dirty) {
            $dirty | ForEach-Object { Write-Host $_ }
            Fail '上のファイルにコミットしていない変更があります。コミットするか、git checkout -- <ファイル> で取り消してから実行してください。'
        }
        Write-Host '最新の main を取り込んでいます'
        Invoke-Native 'git' @('pull', '--ff-only')
    } elseif ($Build) {
        $branch = git branch --show-current
        Write-Host "-NoPull が指定されたので、今のチェックアウト（$branch）をそのままビルドします"
    } else {
        $branch = git branch --show-current
        Write-Host "-NoPull が指定されたので、今のチェックアウト（$branch）のコミットのビルドを取ってきます"
    }

    if ($Build) {
        Write-Host 'marimo-hook をビルドしています'
        Invoke-Native 'cargo' @('build', '--release', '-p', 'marimo-hook')

        # npm.ps1 を経由すると PowerShell が引数の -- を取り除くことがあるので、npm.cmd を直接呼ぶ。
        # npm install は npm の版や OS によって package-lock.json を書き直し、次の実行を変更ありで止めてしまう。
        Push-Location -LiteralPath (Join-Path $root 'app')
        try {
            Write-Host 'アプリの依存をインストールしています'
            Invoke-Native 'npm.cmd' @('ci', '--no-audit', '--no-fund')

            Write-Host 'アプリをビルドしています'
            Invoke-Native 'npm.cmd' @('run', 'tauri', '--', 'build', '--bundles', 'nsis')
        } finally {
            Pop-Location
        }

        $hook = Join-Path $target 'release\marimo-hook.exe'
        if (-not (Test-Path -LiteralPath $hook)) { Fail "ビルドした marimo-hook が見つかりません: $hook" }
        $nsisDir = Join-Path $target 'release\bundle\nsis'
        $installer = Get-ChildItem -LiteralPath $nsisDir -Filter 'marimo_*_x64-setup.exe' -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending |
            Select-Object -First 1
        if (-not $installer) { Fail "ビルドしたインストーラが見つかりません: $nsisDir" }
        $retryInstall = "$hook install を実行し直してください"
    } else {
        $work = Join-Path ([IO.Path]::GetTempPath()) ('marimo-update-' + [Guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $work | Out-Null
        $extract = Get-Build $work
        $hook = Join-Path $extract 'marimo-hook.exe'
        if (-not (Test-Path -LiteralPath $hook)) { Fail "取ってきたものの中に marimo-hook.exe が見つかりません。" }
        $installer = @(Get-ChildItem -LiteralPath $extract -Filter 'marimo_*_x64-setup.exe') | Select-Object -First 1
        if (-not $installer) { Fail "取ってきたものの中にインストーラが見つかりません。" }
        # 一時フォルダは終わると消えるので、install をやり直すときはスクリプトごと実行し直してもらう。
        $retryInstall = 'tools\update.ps1 をもう一度実行してください'
    }

    $running = @(Get-InstalledMarimo $installDir)
    if ($running.Count -gt 0) {
        Write-Host '起動中の marimo を終了しています'
        foreach ($p in $running) {
            if ($p.HasExited) { continue }
            try {
                $p.Kill()
            } catch {
                Write-Host "marimo（PID $($p.Id)）を終了できませんでした: $($_.Exception.Message)"
            }
        }
        # 終了させたプロセスは、一覧を取り直すと片付く前の姿がまだ見えることがあるので、同じオブジェクトの終了を待つ。
        # それでも残るときは、動いているアプリを自分で閉じるインストーラに任せて先へ進む。
        $left = @($running | Where-Object { -not $_.WaitForExit(10000) })
        if ($left.Count -gt 0) {
            Write-Host 'marimo がまだ終了していないので、インストーラに終了を任せます'
        }
    }

    Write-Host "$($installer.Name) でアプリを入れ直しています"
    $setup = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -Wait -PassThru
    if ($setup.ExitCode -ne 0) {
        Fail "インストーラが失敗しました（終了コード $($setup.ExitCode)）。marimo を終了できなかった場合に起こるので、立ち絵を右クリックして「終了」を選んでから、もう一度実行してください。"
    }
    if (-not (Test-Path -LiteralPath $installedExe)) {
        Fail "インストーラは成功を返しましたが、$installedExe が見つかりません。"
    }

    Write-Host 'marimo-hook install を実行しています'
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $hook install
    } finally {
        $ErrorActionPreference = $saved
    }
    $installOk = ($LASTEXITCODE -eq 0)

    Write-Host 'marimo を起動しています'
    Start-Process -FilePath $installedExe

    if (-not $installOk) {
        Fail "marimo-hook install が失敗しました。アプリはすでに新しいものに入れ直して起動しています。上のメッセージを確かめてから、$retryInstall。"
    }
    Write-Host '更新が終わりました'
} finally {
    Pop-Location
    if ($work) { Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue }
}
