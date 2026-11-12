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

- コメントは日本語で、目的・設計意図・制約を書く。UI の文言も日本語のみ。
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

- ゲーム内の確認は、`Mods\CityCheckup\` に置く目印ファイル（`debug-report`、`debug-missing`、`debug-delete`、`debug-speed3`。`mod/src/CityCheckup/DebugMarkers.cs`）で UI 操作を代替し、`Player.log` と画面保存で結果を見る。通常の利用者環境には置かない。
- 実セーブでの削除確認は、自動保存で上書きされる前にゲームを止め、保存一覧の前後を比較する。
