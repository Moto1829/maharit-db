# MaharitDB インストールスクリプト（Windows PowerShell 向け）
#
# 使い方:
#   irm https://raw.githubusercontent.com/Moto1829/maharit-db/main/install.ps1 | iex
#   または
#   .\install.ps1 [-Version <version>] [-InstallDir <dir>] [-NoConfirm]
#
# パラメーター:
#   -Version <string>    インストールするバージョン（例: v0.2.0）。省略時は最新
#   -InstallDir <string> インストール先ディレクトリ（デフォルト: $env:LOCALAPPDATA\MaharitDB\bin）
#   -NoConfirm           確認プロンプトをスキップ

[CmdletBinding()]
param(
    [string]$Version    = "",
    [string]$InstallDir = "$env:LOCALAPPDATA\MaharitDB\bin",
    [switch]$NoConfirm
)

$ErrorActionPreference = "Stop"

$Repo       = "Moto1829/maharit-db"
$BinaryName = "maharit.exe"
$Target     = "x86_64-pc-windows-msvc"

# ── カラー出力ヘルパー ───────────────────────────────────────────────────────
function Write-Info    { param([string]$Msg) Write-Host "[INFO]  $Msg" -ForegroundColor Cyan }
function Write-Ok      { param([string]$Msg) Write-Host "[OK]    $Msg" -ForegroundColor Green }
function Write-Warn    { param([string]$Msg) Write-Host "[WARN]  $Msg" -ForegroundColor Yellow }
function Write-Err     { param([string]$Msg) Write-Host "[ERROR] $Msg" -ForegroundColor Red; exit 1 }

# ── 最新バージョン取得 ───────────────────────────────────────────────────────
if (-not $Version) {
    Write-Info "最新バージョンを確認中..."
    try {
        $ApiUrl   = "https://api.github.com/repos/$Repo/releases/latest"
        $Response = Invoke-RestMethod -Uri $ApiUrl -UseBasicParsing -Headers @{ "User-Agent" = "maharit-installer" }
        $Version  = $Response.tag_name
    } catch {
        Write-Err "最新バージョンを取得できませんでした: $_"
    }
    if (-not $Version) { Write-Err "バージョン情報が空です" }
}

# v プレフィックス正規化
if (-not $Version.StartsWith("v")) { $Version = "v$Version" }

$ArchiveName  = "maharit-$Version-$Target.zip"
$DownloadUrl  = "https://github.com/$Repo/releases/download/$Version/$ArchiveName"
$ChecksumUrl  = $DownloadUrl -replace '\.zip$', '.sha256'

Write-Host ""
Write-Host "MaharitDB インストーラー" -ForegroundColor White -BackgroundColor DarkBlue
Write-Host "  バージョン  : $Version"
Write-Host "  ターゲット  : $Target"
Write-Host "  インストール先: $InstallDir\$BinaryName"
Write-Host ""

if (-not $NoConfirm) {
    $Confirm = Read-Host "インストールを続行しますか？ [y/N]"
    if ($Confirm -notmatch '^[yY]') {
        Write-Info "キャンセルしました"
        exit 0
    }
}

# ── ダウンロード ─────────────────────────────────────────────────────────────
$TmpDir      = [System.IO.Path]::Combine([System.IO.Path]::GetTempPath(), "maharit-install-$([System.IO.Path]::GetRandomFileName())")
$ArchivePath = Join-Path $TmpDir $ArchiveName

New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

Write-Info "ダウンロード中: $DownloadUrl"
try {
    $ProgressPreference = "SilentlyContinue"
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $ArchivePath -UseBasicParsing
    $ProgressPreference = "Continue"
} catch {
    Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
    Write-Err "ダウンロードに失敗しました: $_"
}

# ── チェックサム検証 ──────────────────────────────────────────────────────────
$ChecksumPath = Join-Path $TmpDir "checksum.sha256"
try {
    Invoke-WebRequest -Uri $ChecksumUrl -OutFile $ChecksumPath -UseBasicParsing -ErrorAction Stop
    Write-Info "チェックサムを検証中..."
    $Expected = (Get-Content $ChecksumPath -Raw).Trim().Split(" ")[0].ToLower()
    $Actual   = (Get-FileHash -Path $ArchivePath -Algorithm SHA256).Hash.ToLower()
    if ($Expected -ne $Actual) {
        Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
        Write-Err "チェックサムの検証に失敗しました`n  期待値: $Expected`n  実際値: $Actual"
    }
    Write-Ok "チェックサム検証 OK"
} catch {
    Write-Warn "チェックサムファイルを取得できませんでした。スキップします"
}

# ── 展開 & インストール ──────────────────────────────────────────────────────
Write-Info "展開中..."
Expand-Archive -Path $ArchivePath -DestinationPath $TmpDir -Force

$BinaryPath = Join-Path $TmpDir $BinaryName
if (-not (Test-Path $BinaryPath)) {
    # アーカイブ直下以外のケース
    $Found = Get-ChildItem -Recurse $TmpDir -Filter $BinaryName | Select-Object -First 1
    if ($Found) { $BinaryPath = $Found.FullName }
    else {
        Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
        Write-Err "アーカイブにバイナリが見つかりません: $BinaryName"
    }
}

if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

$Destination = Join-Path $InstallDir $BinaryName
Copy-Item -Path $BinaryPath -Destination $Destination -Force

Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue

# ── PATH 設定 ────────────────────────────────────────────────────────────────
$CurrentPath = [System.Environment]::GetEnvironmentVariable("PATH", "User")
if ($CurrentPath -notlike "*$InstallDir*") {
    Write-Info "ユーザー PATH に $InstallDir を追加中..."
    [System.Environment]::SetEnvironmentVariable(
        "PATH",
        "$InstallDir;$CurrentPath",
        "User"
    )
    Write-Warn "PATH 変更を反映するにはターミナルを再起動してください"
} else {
    Write-Info "PATH は既に設定されています"
}

# ── 確認 ────────────────────────────────────────────────────────────────────
if (Test-Path $Destination) {
    Write-Ok "インストール完了: $Destination"
    try {
        $InstalledVer = & $Destination --version 2>&1
        if ($InstalledVer) { Write-Host "  $InstalledVer" }
    } catch {}
} else {
    Write-Err "インストールに失敗しました"
}

Write-Host ""
Write-Host "MaharitDB $Version のインストールが完了しました！" -ForegroundColor Green
Write-Host "  使い方: maharit --help"
