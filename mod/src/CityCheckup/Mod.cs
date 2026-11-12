using Game;
using Game.Modding;
using Game.Simulation;
using CityCheckup.Analysis;
using CityCheckup.Missing;
using CityCheckup.Ui;
using UnityEngine;

namespace CityCheckup;

/// <summary>
/// mod の入口。ゲームが Mods フォルダーの DLL を読み込むと OnLoad が呼ばれ、ここで自前のシステムを登録する。
/// </summary>
/// <remarks>
/// ゲームの処理を変えるシステムはここで登録しない方針。分析・表示は読み取り専用、
/// 削除はプレイヤーが UI で選んだときだけ行う（後続の機能で追加する）。
/// </remarks>
public sealed class Mod : IMod
{
    /// <summary>Player.log でこの mod の行を見分けるための接頭辞。</summary>
    public const string LogPrefix = "[CityCheckup]";

    public void OnLoad(UpdateSystem updateSystem)
    {
        // UI のバインディングは UIUpdate の段で更新する（ゲーム本体の UI システムと同じ段）。
        updateSystem.UpdateAt<PanelUISystem>(SystemUpdatePhase.UIUpdate);
        // 欠落アセットの検出と削除。探す・消すはプレイヤーが UI で選んだときだけ動く。
        updateSystem.UpdateAt<MissingAssetSystem>(SystemUpdatePhase.UIUpdate);
        // 経路探しの依頼数は、PathfindSetupSystem の直前・直後で読む（パネルを開いている間だけ数える）。
        updateSystem.UpdateBefore<PathfindDemandBeforeSystem, PathfindSetupSystem>(SystemUpdatePhase.MainLoop);
        updateSystem.UpdateAfter<PathfindDemandSystem, PathfindSetupSystem>(SystemUpdatePhase.MainLoop);
        Debug.Log($"{LogPrefix} loaded");
    }

    public void OnDispose()
    {
    }
}
