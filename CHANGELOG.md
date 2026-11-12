# Changelog

City Checkup（Cities: Skylines II 用 mod）の変更履歴です。各版の節は Paradox Mods の変更履歴（ChangeLog）にもそのまま使われるため、
英語と日本語の両方で書きます（`mod/scripts/Release-ParadoxMods.ps1` が `## [版]` の節を取り出して送ります）。

## [0.1.0] - 2026-11-12

- First release on Paradox Mods (ID 167225).
- Load analysis: splits the slowdown into waiting for pathfinding and simulation frame time, shows pathfinding work and success rate per request type, and lists districts short of jobs.
- Missing assets: finds objects left by uninstalled mods, jumps to them and deletes the selected ones.
- The panel follows the game language (Japanese or English).
- 初回公開。負荷分析（遅れの原因の切り分け・経路探しの作業量・職の足りない地区）と、欠落アセットの検出・削除。ゲームの言語に合わせて日本語／英語で表示。
