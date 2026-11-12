# AGENTS.md

City Checkup のリポジトリ固有の作業規約。設計の背景は [DESIGN.md](DESIGN.md)、利用者向けの手順は [README.md](README.md) を参照する。

## 構成

- `mod/`: Cities: Skylines II 用のコード mod（C# + Bun/React UI）。製品の主軸。
  - `mod/src/CityCheckup.Core`: ゲーム型を使わない判定・集計・JSON 生成（`netstandard2.1`）。
  - `mod/src/CityCheckup`: ゲームのシステム・バインディング・mod 入口（`netstandard2.1`）。Core のソースを取り込んで DLL 1 つで配る。
  - `mod/tests/CityCheckup.Core.Tests`: Core の MSTest（`net11.0`）。
  - `mod/ui`: ゲーム内パネルの UI（TypeScript/React、Bun で `CityCheckup.mjs` にビルド）。
- `src/`（クレート名 `cok-inspector`）: `.cok` セーブをオフラインで調べ、別名保存で整理する Rust ツール。Windows GUI（`src/windows.rs`）と CLI（`src/bin/`）。
- `tools/`: 計測専用の一時 mod とパッチ（`pathfind-count`、`reference-capture`）。製品には含めない。

## コマンド

```text
# Rust（リポジトリ直下）
cargo build --release
cargo test

# mod の Core テスト
dotnet test mod/tests/CityCheckup.Core.Tests

# UI（mod/ui で実行）
bun install
bun run typecheck
bun run build            # dist/CityCheckup.mjs を作る

# mod 本体（UI を先にビルドしておくと .mjs が出力先へコピーされる）
dotnet build mod/src/CityCheckup/CityCheckup.csproj -c Release -p:GameManaged=<Cities2_Data\Managed のコピー>
```

- mod 本体のビルドにはゲームの `Managed` フォルダーの DLL が要る。既定の参照先 `target/size-investigation/ref` は Git 管理外なので、`-p:GameManaged` で明示するのが確実。
- `bun.lock` を正本にする。UI の依存は `typescript` のみ。React と `cs2/api` はゲームが `window` に提供する。

## 規約

- コメントは日本語で、目的・設計意図・制約を書く。UI の文言は日本語と英語の両方（`mod/ui/src/i18n.ts`、所見の文は Core の `LoadReport`）で、片方だけ足さない。
- `Directory.Build.props` で `TreatWarningsAsErrors` と `EnforceCodeStyleInBuild` が有効。警告を残さない。`Version` は `/vava` 経由でだけ更新する。
- ゲームの処理を変えるシステムは登録しない。分析・表示は読み取り専用で、削除はプレイヤーが UI で選んだときだけ行う。
- 判定ロジックはゲーム型に依存しない Core に置き、`CityCheckup.Core.Tests` で検証する。ゲーム側のシステムは薄く保つ。
- C# の UI バインディング名（`PanelUISystem.Group` = `cityCheckup` 配下）と `mod/ui/src` は対で、名前を変えるときは両方を直す。
- 重い集計は UI を開いている間、または「探す」操作の間だけ動かす。`UISystemBase` は読み込み画面中も動くため、`OnGameLoadingComplete` 後に処理を始める。
- Gameface（ゲーム内 UI）では `toLocaleString` の書式指定が効かないので数値は手で整形し、隣り合う JSX の文字ノードは 1 つのテンプレート文字列にまとめる。
- Rust の編集ツールは入力 `.cok` を書き換えない。出力は別名の `.cok` と `.cok.cid` で、出力先が既にあればエラーにする。参照記録との不一致・未対応の構造・市民や世帯と重なる候補があれば書き出しを中止する。
- macOS Intel はサポート対象外。
- `.gitignore` は C# の `bin/`・`obj/` を `mod/`・`tools/` 配下に限定している。`src/bin/`（Rust の CLI 入口）を無視しないこと。

## 動作確認

- ゲーム内の確認は、`Mods\CityCheckup\` に置く目印ファイル（`debug-report`、`debug-missing`、`debug-delete`、`debug-speed3`、英語表示に固定する `debug-lang-en`。`mod/src/CityCheckup/DebugMarkers.cs`）で UI 操作を代替し、`Player.log` と画面保存で結果を見る。通常の利用者環境には置かない。
- 実セーブでの削除確認は、自動保存で上書きされる前にゲームを止め、保存一覧の前後を比較する。
- 確認用に `Mods\CityCheckup\` へ開発版を直接置くと、購読している公開版と二重に読み込まれる。確認の間は購読を無効にし、終わったら直接置きを片付けて購読へ戻す。

## リリース（/vava）

製品は City Checkup の mod で、配布先は Paradox Mods（ModId `167225`、https://mods.paradoxplaza.com/mods/167225/Windows ）。
Rust ツール（`Cargo.toml` の `cok-inspector`）は配布しておらず、版はリリースと連動させない。

- **版の正本（PRIMARY）**: `mod/Directory.Build.props` の `<Version>`。
- **同期対象**: `mod/src/CityCheckup/Properties/PublishConfiguration.xml` の `<ModVersion Value>`。ほかに版を持つ file は無い（`mod/ui/package.json` は版を持たない）。
- **変更履歴**: リポジトリ直下の `CHANGELOG.md` に `## [版] - 日付` の節を足す。英語と日本語の両方で書く（その節が Paradox Mods の ChangeLog として送られる）。
- **公開**: PowerShell 7 で `& ./mod/scripts/Release-ParadoxMods.ps1 -Command NewVersion` をリポジトリ直下で実行する（`pwsh -File` の入れ子起動は削除検査フックが拒否するため、今のセッションで直接呼ぶ）。
  - 版の照合 → UI と mod 本体の Release ビルド → DLL の版と SHA-256 の検証 → 公開用のゲーム PC へ送付してハッシュ照合 → ゲーム付属の ModPublisher で公開、の順に 1 本で行う。
  - 公開用のゲーム PC は Paradox Mods の購読で City Checkup を使う（2026-11-12 に直接置きの `Mods\CityCheckup` を撤去）。直接置きと購読が両方あると二重に読み込まれるため、リリースで直接置きは更新しない。`-UpdateLocalInstall` は購読していない PC でだけ使う。
  - 公開前の確認だけをしたいときは `-DryRun`（ビルド・検証・送付まで）。
  - 成功の判定は ModPublisher の出力の `... process finished successfully`（初回は `Mod publishing process`、版の更新は `Publishing new version updating process`。受付応答）。公開後のページの再取得や反映待ちはしない。
  - 説明文・画像だけを更新するときは `-Command Update`。初回公開（`Publish`）は済んでいる。
- **環境固有の値**: SSH の接続先・ゲームのパス・参照 DLL の場所は Git 管理外の `mod/scripts/release.local.json` に置く（書式は `release.local.example.json`）。無ければスクリプトが止まる。
- **公開用のゲーム PC は本人が普段遊ぶ PC**。スクリプトはゲームが起動中なら何もせずに中止する。起動中で止まったら、ゲームを終了してよいか本人に確認してから再実行する。ゲームを勝手に終了しない。
- ModPublisher はゲームでログインした Paradox アカウントで自動ログインする。パスワードは扱わない。ゲーム PC に .NET 6 が無くても `DOTNET_ROLL_FORWARD=Major` で動く（スクリプトが設定する）。
- **deploy**: なし（`DEPLOY_REQUIREMENT=not-applicable`）。Paradox Mods への公開そのものが配布。
- **署名・CI**: なし。ローカルのスクリプトだけで公開する。依存の更新は `.github/dependabot.yml`（cargo・nuget はテストのみ・bun、週次で minor/patch をまとめる）が PR にする。
- **告知**: Paradox Mods 上で一般公開になっている版が、公開後の X 告知の対象になる。1.0.0 はログインなしでページを開ける状態で、本人の了承のもと公開範囲を変えずに X で告知した（@KagayoiJP）。公開範囲が変わった場合は本人に確認する。
- **既知の経緯**: 0.1.0 は 2026-11-12 に release branch を作らずに公開した（公開時のコミットは `2692b58`）。1.0.0 以降は `release/<版>` branch を作る（`release/1.0.0` がある）。
