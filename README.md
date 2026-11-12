# CityCheckup

Cities: Skylines II の街を診断する mod（City Checkup、街の診断）と、`.cok` セーブをオフラインで整理する Windows ツールのリポジトリです。設計は [DESIGN.md](DESIGN.md)、開発者向けの手順は [AGENTS.md](AGENTS.md) にあります。

## ゲーム内 mod「City Checkup」

画面左上の「街の診断」ボタンからパネルを開きます。画面表示は日本語のみです。

- **負荷分析**: 経路探しを多く頼んでいる処理（探索の作業量と成功率）、設定した速度と実際のシミュレーション速度、速度が落ちる原因（経路探し待ち／1 フレームの処理時間）、学歴別の求職者と空き職場、職の足りない地区を表示します。地区は「見に行く」でカメラを移せます。集計はこのタブを開いている間だけ動きます。
- **欠落アセット**: アンインストールした mod の物を探して名前ごとに一覧し、選んだものを削除します。道路・線路・エリアは一覧と「見に行く」だけで、削除できません。削除は元に戻せないので、やり直すときは保存せずに街を読み込み直してください。

### 導入

Paradox Mods への掲載は未完了です。ビルドして次のフォルダーへ `CityCheckup.dll` と `CityCheckup.mjs` を置くと読み込まれます。

```text
%UserProfile%\AppData\LocalLow\Colossal Order\Cities Skylines II\Mods\CityCheckup\
```

ビルドにはゲームの `Cities2_Data\Managed` の DLL（またはそのコピー）が必要です。

```text
cd mod\ui
bun install
bun run build
cd ..\..
dotnet build mod\src\CityCheckup\CityCheckup.csproj -c Release -p:GameManaged=<Managedフォルダー>
```

動作確認したゲームのバージョンは 1.6.4f1 です。

## Windowsアプリ（セーブの整理）

「セーブを開く…」から `.cok` を選ぶと、「消すもの」と「残すもの」が表示されます。実行ファイルに `.cok` を渡して直接開くこともできます。

Windows の Rust ツールチェーンで `cargo build --release` を実行すると、`target\release\cok-inspector.exe` が生成されます。

### 削除候補の確認と別名保存

「保存の準備」に読み込み記録とゲームの部品を指定すると、「別名で保存…」が使えるようになります。読み込み記録は対象セーブから採取した `.bin` と、それに対応する `.bin.stride` の組です。ゲームの部品にはゲーム版に対応する `PipelinePlugin.dll` を指定します。

車両は、対応するレコード形式で、行内の全車両が再生成条件を満たす場合に削除します。市民や世帯と重なる削除候補、分割に未対応のコンポーネント、参照記録との不一致がある場合は書き出しを中止します。

従来のCLI名 `clean-beartris` でも同じ編集処理を実行できます。入力セーブだけを渡すと候補を表示します。保存時は別名の `.cok` と `.cok.cid` を作ります。4つの出力IDはそれぞれ異なる32文字の16進数を渡します。

```text
cargo run --release --bin clean-beartris -- <元cok> <出力cok> <参照記録> <stride記録> <PipelinePlugin.dll> <package-id> <data-id> <metadata-id> <texture-id>
```

`tools/reference-capture` に計測コードがあります。ゲームと同じ `mscorlib.dll`、`netstandard.dll`、`UnityEngine.CoreModule.dll` を参照して `ReferenceCapture.cs` をライブラリにコンパイルします。Patcher は `-p:CecilPath=<Colossal.Mono.Cecil.dllの絶対パス>` でビルドし、`<元のColossal.Core.dll> <計測DLL> <出力DLL>` の順に引数を渡します。計測用プロセスは `CS2_REFERENCE_CAPTURE` を新規ファイルの絶対パスに設定し、Burst を無効にして対象セーブだけをロードします。参照ログと同名の `.stride` が一緒に出力されます。
