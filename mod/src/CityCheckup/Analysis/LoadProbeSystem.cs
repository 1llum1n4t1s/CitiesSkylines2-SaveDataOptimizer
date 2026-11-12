using System.Collections.Generic;
using CityCheckup.Core;
using Game;
using Game.Pathfind;
using Game.Simulation;

namespace CityCheckup.Analysis;

/// <summary>
/// 負荷の原因を測るため、ゲーム自身が持っている値を画面のフレームごとに読む（ゲームの処理は変えない、読み取り専用）。
/// </summary>
/// <remarks>
/// 読む値はどれもゲームの公開プロパティで、反射は使わない（ゲームの更新で壊れにくい）。
/// - SimulationSystem.selectedSpeed / smoothSpeed / frameDuration / frameIndex: 設定速度、実際の速度、1 フレームの処理時間
/// - PathfindResultSystem.pendingSimulationFrame / pendingRequestCount: 経路探しの結果待ち（ゲームはこれで速度を落とす）
/// - PathfindResultSystem.queryStats: 依頼元のシステムごとの累計（件数・成功数・探索の作業量）。区間の始めと終わりの差を取る。
/// 毎フレームの処理は数個の値を足すだけで軽い。queryStats の読み取りは区間の始めと終わりだけ。
/// パネルを開いている間（<see cref="Collecting"/>）だけ動かす。
/// </remarks>
public sealed partial class LoadProbeSystem : GameSystemBase
{
    private SimulationSystem m_Simulation = null!;
    private PathfindResultSystem m_PathfindResults = null!;
    private readonly SpeedSampler m_Speed = new();
    private PathfindSnapshot? m_Start;
    private double m_StartSeconds;

    /// <summary>パネルを開いている間だけ true にする（PanelUISystem が切り替える）。</summary>
    public bool Collecting { get; set; }

    protected override void OnCreate()
    {
        base.OnCreate();
        m_Simulation = World.GetOrCreateSystemManaged<SimulationSystem>();
        m_PathfindResults = World.GetOrCreateSystemManaged<PathfindResultSystem>();
    }

    protected override void OnUpdate()
    {
        if (!Collecting || m_Start is null)
        {
            return;
        }
        var factor = SpeedSampler.PathfindSpeedFactor(m_PathfindResults.pendingSimulationFrame, m_Simulation.frameIndex);
        m_Speed.Add(
            UnityEngine.Time.unscaledDeltaTime,
            m_Simulation.selectedSpeed,
            m_Simulation.smoothSpeed,
            factor,
            m_Simulation.frameDuration,
            m_PathfindResults.pendingRequestCount);
    }

    /// <summary>区間を始める（パネルを開いたとき・街を読み込んだとき）。</summary>
    public void Begin()
    {
        m_Speed.Reset();
        m_StartSeconds = UnityEngine.Time.realtimeSinceStartup;
        m_Start = Snapshot(m_StartSeconds);
    }

    /// <summary>区間を締めて結果を返し、次の区間を始める。まだ始まっていなければ両方 null。</summary>
    public (PathfindWindow? Pathfind, SpeedWindow? Speed) Close()
    {
        if (m_Start is null)
        {
            Begin();
            return (null, null);
        }
        var now = UnityEngine.Time.realtimeSinceStartup;
        var end = Snapshot(now);
        var pathfind = end.Since(m_Start);
        var speed = m_Speed.Close(now - m_StartSeconds);
        m_Start = end;
        m_StartSeconds = now;
        return (pathfind, speed);
    }

    /// <summary>ゲームの累計（queryStats）を、依頼元のシステム名ごとにまとめて写し取る。</summary>
    private PathfindSnapshot Snapshot(double seconds)
    {
        var bySystem = new Dictionary<string, PathfindCounter>();
        foreach (var pair in m_PathfindResults.queryStats)
        {
            var name = pair.Key.m_System?.GetType().Name ?? string.Empty;
            var c = new PathfindCounter(pair.Value.m_QueryCount, pair.Value.m_SuccessCount, pair.Value.m_GraphTraversal);
            bySystem[name] = bySystem.TryGetValue(name, out var sum) ? sum.Plus(c) : c;
        }
        return new PathfindSnapshot(bySystem, m_Simulation.frameIndex, seconds);
    }
}
