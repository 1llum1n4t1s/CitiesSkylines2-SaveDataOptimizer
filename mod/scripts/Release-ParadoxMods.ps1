#requires -Version 7
<#
.SYNOPSIS
  City Checkup をビルドし、配布物を検証して Paradox Mods へアップロードする（/vava の公開経路）。

.DESCRIPTION
  開発 PC で実行する。ゲームと Paradox アカウントのログインがある「公開用のゲーム PC」へ SSH で配布物を送り、
  そこにあるゲーム付属の ModPublisher で公開する（Paradox Mods への公開はゲームでログインした状態を使うため）。

  手順:
    1. 版の照合: mod/Directory.Build.props の <Version>（正本）と PublishConfiguration.xml の ModVersion が一致し、
       CHANGELOG.md に同じ版の節があることを確かめる。NewVersion / Update では ModId が必要。
    2. ビルド: UI（bun）→ mod 本体（dotnet, Release）。
    3. 配布物の検証: DLL のアセンブリ版が正本の版と一致し、.mjs があることを確かめ、SHA-256 を記録する。
    4. ゲーム PC の確認: ゲームが起動中なら何もせずに中止する（遊んでいる最中のゲームを邪魔しないため）。
    5. 送付: 専用の作業フォルダーへ配布物・公開設定・画像を送り、ハッシュが一致することを確かめる。
       公開設定の ChangeLog は CHANGELOG.md の該当節から作る（二重管理しないため）。
    6. 公開: ModPublisher を実行し、「finished successfully」と ModId を確認する（-DryRun では実行しない）。
    7. 直接置きの更新（-UpdateLocalInstall のときだけ）: ゲーム PC の Mods\CityCheckup を同じ配布物へ置き換える。
       公開用のゲーム PC は Paradox Mods の購読で使っており、直接置きと購読が両方あると二重に読み込まれるため、既定では行わない。

  環境固有の値（SSH の接続先・ゲームのパス・参照 DLL の場所）は、Git 管理外の mod/scripts/release.local.json に置く。
  書式は mod/scripts/release.local.example.json を参照。

.PARAMETER Command
  ModPublisher の命令。初回は Publish、版の更新は NewVersion、説明文や画像だけの更新は Update。

.PARAMETER DryRun
  ビルド・検証・送付まで行い、アップロードと直接置きの更新はしない。

.PARAMETER UpdateLocalInstall
  公開後に、ゲーム PC の Mods\CityCheckup（直接置き）も同じ配布物へ置き換える。購読で使っている PC では指定しない。
#>
param(
    [ValidateSet('Publish', 'NewVersion', 'Update')]
    [string]$Command = 'NewVersion',
    [switch]$DryRun,
    [switch]$UpdateLocalInstall
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$modRoot = Split-Path -Parent $PSScriptRoot
$repoRoot = Split-Path -Parent $modRoot
$project = Join-Path $modRoot 'src\CityCheckup'
$properties = Join-Path $project 'Properties'
$configPath = Join-Path $properties 'PublishConfiguration.xml'
$changelogPath = Join-Path $repoRoot 'CHANGELOG.md'
$localConfigPath = Join-Path $PSScriptRoot 'release.local.json'

function Step([string]$text) { Write-Host "== $text" -ForegroundColor Cyan }

# ---- 環境固有の設定 ----
if (-not (Test-Path -LiteralPath $localConfigPath)) {
    throw "環境固有の設定がありません: $localConfigPath（release.local.example.json を写して値を入れる）"
}
$local = Get-Content -Raw -LiteralPath $localConfigPath | ConvertFrom-Json
foreach ($key in 'sshHost', 'gameDir', 'userDataDir', 'gameManaged') {
    if ([string]::IsNullOrWhiteSpace($local.$key)) { throw "release.local.json に $key がありません" }
}
$workDir = if ($local.workDir) { $local.workDir } else { Join-Path ([IO.Path]::GetTempPath()) 'CityCheckup-release' }

# ---- 1. 版の照合 ----
Step '版の照合'
[xml]$props = Get-Content -Raw -LiteralPath (Join-Path $modRoot 'Directory.Build.props')
$version = ($props.Project.PropertyGroup | ForEach-Object { $_.Version } | Where-Object { $_ }) | Select-Object -First 1
if (-not $version) { throw 'Directory.Build.props に Version がありません' }
[xml]$publish = Get-Content -Raw -LiteralPath $configPath
$modVersion = $publish.Publish.ModVersion.Value
$modId = $publish.Publish.ModId.Value
if ($modVersion -ne $version) { throw "PublishConfiguration.xml の ModVersion ($modVersion) が正本の版 ($version) と違います" }
if ($Command -ne 'Publish' -and [string]::IsNullOrWhiteSpace($modId)) { throw "$Command には PublishConfiguration.xml の ModId が必要です" }
if ($Command -eq 'Publish' -and -not [string]::IsNullOrWhiteSpace($modId)) { throw "ModId ($modId) が既にあります。版の更新は NewVersion を使います" }

# CHANGELOG.md の「## [版]」節を取り出す（次の「## 」見出しまで）。
$changelog = Get-Content -LiteralPath $changelogPath -Encoding utf8
$start = [array]::FindIndex($changelog, [Predicate[string]] { param($l) $l -match "^## \[$([regex]::Escape($version))\]" })
if ($start -lt 0) { throw "CHANGELOG.md に [$version] の節がありません" }
$end = [array]::FindIndex($changelog, $start + 1, [Predicate[string]] { param($l) $l -match '^## ' })
if ($end -lt 0) { $end = $changelog.Length }
$notes = ($changelog[($start + 1)..($end - 1)] | ForEach-Object { $_.TrimStart() }) -join "`n"
$notes = $notes.Trim()
if (-not $notes) { throw "CHANGELOG.md の [$version] の節が空です" }
"版 $version / ModId $(if ($modId) { $modId } else { '(初回)' }) / 命令 $Command"

# ---- 2. ビルド ----
Step 'ビルド'
Push-Location (Join-Path $modRoot 'ui')
try {
    bun install --frozen-lockfile
    bun run build
} finally { Pop-Location }
dotnet build (Join-Path $project 'CityCheckup.csproj') -c Release "-p:GameManaged=$($local.gameManaged)" --nologo -v q

# ---- 3. 配布物の検証 ----
Step '配布物の検証'
$out = Join-Path $project 'bin\Release\netstandard2.1'
$dll = Join-Path $out 'CityCheckup.dll'
$mjs = Join-Path $out 'CityCheckup.mjs'
foreach ($f in $dll, $mjs) { if (-not (Test-Path -LiteralPath $f) -or (Get-Item -LiteralPath $f).Length -eq 0) { throw "配布物がありません: $f" } }
$assemblyVersion = [Reflection.AssemblyName]::GetAssemblyName($dll).Version
$expected = [version]$version
if ($assemblyVersion.Major -ne $expected.Major -or $assemblyVersion.Minor -ne $expected.Minor -or $assemblyVersion.Build -ne $expected.Build) {
    throw "DLL の版 ($assemblyVersion) が $version と違います"
}
$hashes = [ordered]@{}
foreach ($f in $dll, $mjs) { $hashes[(Split-Path -Leaf $f)] = (Get-FileHash -LiteralPath $f -Algorithm SHA256).Hash }
$hashes.GetEnumerator() | ForEach-Object { '{0} {1}' -f $_.Key, $_.Value }

# 公開設定の ChangeLog を CHANGELOG.md の節で置き換えた写しを作る（複数行の欄は行頭に空白を入れない）。
New-Item -ItemType Directory -Force -Path $workDir | Out-Null
$stagedConfig = Join-Path $workDir 'PublishConfiguration.xml'
$text = Get-Content -Raw -LiteralPath $configPath -Encoding utf8
$escaped = [Security.SecurityElement]::Escape($notes)
$text = [regex]::Replace($text, '(?s)<ChangeLog>.*?</ChangeLog>', "<ChangeLog>`n$escaped`n`t</ChangeLog>")
[IO.File]::WriteAllText($stagedConfig, $text, (New-Object Text.UTF8Encoding $false))
[xml](Get-Content -Raw -LiteralPath $stagedConfig) | Out-Null

# ---- 4〜7. ゲーム PC ----
function Remote([string]$script) {
    $enc = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script))
    $lines = ssh -o BatchMode=yes $local.sshHost "powershell.exe -NoProfile -EncodedCommand $enc" 2>&1
    $lines | Where-Object { "$_" -notmatch 'quantum|store now|upgraded' } | ForEach-Object { "$_" }
}
function Quote([string]$s) { "'" + $s.Replace("'", "''") + "'" }

Step 'ゲーム PC の確認'
$state = Remote "if (Get-Process Cities2 -ErrorAction SilentlyContinue) { 'RUNNING' } else { 'IDLE' }"
if ($state -notcontains 'IDLE') { throw "ゲーム PC でゲームが起動中のため中止しました（$state）" }

# ゲーム PC 側の作業フォルダーは、そのユーザーの一時フォルダーの下に版ごとに作る。
$stage = ((Remote "Join-Path `$env:TEMP 'CityCheckup-release\$version'") | Select-Object -Last 1).Trim()
if (-not $stage) { throw 'ゲーム PC の一時フォルダーを取得できませんでした' }
$prep = Remote @"
`$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $(Quote "$stage\content"), $(Quote "$stage\Properties") | Out-Null
'PREP-OK'
"@
if ($prep -notcontains 'PREP-OK') { throw "ゲーム PC の作業フォルダーを作れませんでした: $($prep -join ' ')" }

Step '送付'
$stageScp = $stage.Replace('\', '/')
scp -q -o BatchMode=yes $dll $mjs "$($local.sshHost):$stageScp/content/"
scp -q -o BatchMode=yes $stagedConfig (Join-Path $properties 'Thumbnail.png') (Get-ChildItem -LiteralPath $properties -Filter 'Screenshot-*').FullName "$($local.sshHost):$stageScp/Properties/"
$remoteHashes = Remote "Get-ChildItem -LiteralPath $(Quote "$stage\content") -File | ForEach-Object { '{0} {1}' -f `$_.Name, (Get-FileHash -LiteralPath `$_.FullName -Algorithm SHA256).Hash }"
foreach ($pair in $hashes.GetEnumerator()) {
    if ($remoteHashes -notcontains ('{0} {1}' -f $pair.Key, $pair.Value)) { throw "送付後のハッシュが一致しません: $($pair.Key)" }
}
'ハッシュ一致'

if ($DryRun) {
    Step '-DryRun のためアップロードと直接置きの更新はしません'
    "ゲーム PC の作業フォルダー: $stage"
    return
}

Step "公開（ModPublisher $Command）"
$publisher = "$($local.gameDir)\Cities2_Data\Content\Game\.ModdingToolchain\ModPublisher\ModPublisher.exe"
# ModPublisher は .NET 6 用。ゲーム PC に無くても、入っている新しい .NET で動かす。
$result = Remote @"
`$env:DOTNET_ROLL_FORWARD = 'Major'
Set-Location -LiteralPath $(Quote $stage)
& $(Quote $publisher) $Command $(Quote "$stage\Properties\PublishConfiguration.xml") -c $(Quote "$stage\content") -v 2>&1 | ForEach-Object { "`$_" }
"@
$result | Select-String -Pattern 'Auto logged|Mod published|Mod updated|finished|error|fail|Exception|Id=' | ForEach-Object { $_.Line }
if (-not ($result -match 'finished successfully')) { throw 'ModPublisher が成功を報告しませんでした（上の出力を確認）' }
$published = ($result | Select-String -Pattern 'Id=(\d+)' | Select-Object -First 1)
if ($published) { "ModId: $($published.Matches[0].Groups[1].Value)" }

if ($UpdateLocalInstall) {
    Step '直接置きの mod を更新'
    $mods = "$($local.userDataDir)\Mods\CityCheckup"
    $install = Remote @"
`$ErrorActionPreference = 'Stop'
if (Get-Process Cities2 -ErrorAction SilentlyContinue) { throw 'game running' }
New-Item -ItemType Directory -Force -Path $(Quote $mods) | Out-Null
if (Get-ChildItem -LiteralPath $(Quote $mods) -Filter 'debug-*') { throw 'debug files present' }
Copy-Item -LiteralPath $(Quote "$stage\content\CityCheckup.dll"), $(Quote "$stage\content\CityCheckup.mjs") -Destination $(Quote $mods) -Force
Get-ChildItem -LiteralPath $(Quote $mods) -File | ForEach-Object { '{0} {1}' -f `$_.Name, (Get-FileHash -LiteralPath `$_.FullName -Algorithm SHA256).Hash }
"@
    foreach ($pair in $hashes.GetEnumerator()) {
        if ($install -notcontains ('{0} {1}' -f $pair.Key, $pair.Value)) { throw "直接置きの更新を確認できません: $($install -join ' ')" }
    }
    '直接置きの mod を更新しました'
}
"完了: City Checkup $version（ゲーム PC の作業フォルダー $stage は残しています）"
