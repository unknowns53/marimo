# Windows 用です。macOS では tools/update.sh を使います。
# Windows PowerShell 5.1 は BOM の無いスクリプトを ANSI のコード ページで読んで日本語を壊すので、
# このファイルは BOM 付きの UTF-8 で保存する。
[CmdletBinding()]
param(
    [switch]$NoPull
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

$installDir = Join-Path $env:LOCALAPPDATA 'marimo'
$installedExe = Join-Path $installDir 'marimo.exe'

Push-Location -LiteralPath $root
try {
    if (-not $NoPull) {
        $branch = git branch --show-current
        if ($LASTEXITCODE -ne 0) { Fail 'git でブランチを調べられませんでした。' }
        if ($branch -ne 'main') {
            if (-not $branch) { $branch = '（ブランチなし）' }
            Fail "今のブランチは「$branch」です。main へ切り替えてから実行してください。ほかのブランチを試すときは -NoPull を付けます。"
        }
        $dirty = git status --porcelain --untracked-files=no
        if ($LASTEXITCODE -ne 0) { Fail 'git で変更の有無を調べられませんでした。' }
        if ($dirty) {
            $dirty | ForEach-Object { Write-Host $_ }
            Fail '上のファイルにコミットしていない変更があります。コミットするか、git checkout -- <ファイル> で取り消してから実行してください。'
        }
        Write-Host '最新の main を取り込んでいます'
        Invoke-Native 'git' @('pull', '--ff-only')
    } else {
        $branch = git branch --show-current
        Write-Host "-NoPull が指定されたので、今のチェックアウト（$branch）をそのままビルドします"
    }

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
        Fail "marimo-hook install が失敗しました。アプリはすでに新しいものに入れ直して起動しています。上のメッセージを確かめてから、$hook install を実行し直してください。"
    }
    Write-Host '更新が終わりました'
} finally {
    Pop-Location
}
