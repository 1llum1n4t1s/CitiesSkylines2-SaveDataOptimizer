using System;
using System.Collections.Generic;
using System.Linq;

namespace CityCheckup.Core;

/// <summary>経路探しの依頼元の分類。表示の並び順もこの順にする。</summary>
public enum PathfindSource
{
    JobSearch,
    Shopping,
    ResidentRepath,
    Trips,
    School,
    Home,
    ServiceDispatch,
    VehicleRepath,
    Other,
}

/// <summary>
/// 経路探しの依頼を出したゲームのシステム名から、プレイヤーに分かる分類へ振り分ける。
/// </summary>
/// <remarks>
/// システム名は 2026-11 時点のゲーム（ゆろしてぃでの実測）で確認したもの。
/// ゲームの更新で名前が変わった場合は <see cref="PathfindSource.Other"/> に入るだけで、集計自体は壊れない。
/// ゲームの集計（PathfindResultSystem.queryStats）は依頼元のシステム単位なので、
/// 「失業者の職探し」と「働いている人の転職」は分けられない（ゆろしてぃでは転職は職探し全体の約 3%）。
/// </remarks>
public static class PathfindSources
{
    /// <param name="systemName">依頼を出したシステムの型名（例: FindJobSystem）。</param>
    public static PathfindSource Classify(string systemName)
    {
        switch (systemName)
        {
            case "FindJobSystem":
                return PathfindSource.JobSearch;
            case "ResourceBuyerSystem":
                return PathfindSource.Shopping;
            case "ResidentAISystem":
                return PathfindSource.ResidentRepath;
            case "TripNeededSystem":
            case "LeisureSystem":
            case "TouristFindTargetSystem":
                return PathfindSource.Trips;
            case "FindSchoolSystem":
                return PathfindSource.School;
            case "HouseholdFindPropertySystem":
                return PathfindSource.Home;
        }
        // 配車（タクシー・警察・郵便・ごみ収集など）は「〜DispatchSystem」、
        // 走行中の車両の経路の引き直しは「〜AISystem」という命名で揃っている。
        if (systemName.EndsWith("DispatchSystem", StringComparison.Ordinal))
        {
            return PathfindSource.ServiceDispatch;
        }
        if (systemName.EndsWith("AISystem", StringComparison.Ordinal))
        {
            return PathfindSource.VehicleRepath;
        }
        return PathfindSource.Other;
    }

    /// <summary>UI で使う分類のキー（英字）。表示名は UI 側で日本語にする。</summary>
    public static string Key(PathfindSource source) => source switch
    {
        PathfindSource.JobSearch => "jobSearch",
        PathfindSource.Shopping => "shopping",
        PathfindSource.ResidentRepath => "residentRepath",
        PathfindSource.Trips => "trips",
        PathfindSource.School => "school",
        PathfindSource.Home => "home",
        PathfindSource.ServiceDispatch => "serviceDispatch",
        PathfindSource.VehicleRepath => "vehicleRepath",
        _ => "other",
    };
}

/// <summary>
/// ゲームが依頼元ごとに累計している経路探しの結果（PathfindResultSystem.queryStats の 1 行ぶん）。
/// </summary>
/// <remarks>
/// Work は「探索した道のグラフの量」（m_GraphTraversal をグラフ全体の大きさで割った値）の合計で、
/// 経路探しにかかった実際の作業量の目安になる。件数より正確に「どの依頼が重いか」を表す。
/// </remarks>
public readonly struct PathfindCounter
{
    public PathfindCounter(long queries, long successes, double work)
    {
        Queries = queries;
        Successes = successes;
        Work = work;
    }

    public long Queries { get; }

    /// <summary>行き先が 1 つ以上見つかった件数。</summary>
    public long Successes { get; }

    public double Work { get; }

    public PathfindCounter Plus(PathfindCounter other) => new(Queries + other.Queries, Successes + other.Successes, Work + other.Work);
}

/// <summary>ある時点のゲームの累計（システム名ごと）。区間の差を取るために覚えておく。</summary>
public sealed class PathfindSnapshot
{
    public PathfindSnapshot(IReadOnlyDictionary<string, PathfindCounter> bySystem, uint frame, double seconds)
    {
        BySystem = bySystem;
        Frame = frame;
        Seconds = seconds;
    }

    public IReadOnlyDictionary<string, PathfindCounter> BySystem { get; }

    public uint Frame { get; }

    public double Seconds { get; }

    /// <summary>
    /// 前の時点から今までの区間の増えた分を、依頼元の分類ごとにまとめる。
    /// </summary>
    /// <remarks>
    /// ゲームは街を読み込むと累計を 0 に戻す。前より減っているシステムは、戻ったあとの値をそのまま区間の値とする。
    /// </remarks>
    public PathfindWindow Since(PathfindSnapshot before)
    {
        var bySource = new Dictionary<PathfindSource, PathfindCounter>();
        foreach (var pair in BySystem)
        {
            var now = pair.Value;
            var delta = now;
            if (before.BySystem.TryGetValue(pair.Key, out var then) && now.Queries >= then.Queries)
            {
                delta = new PathfindCounter(now.Queries - then.Queries, now.Successes - then.Successes, Math.Max(0, now.Work - then.Work));
            }
            var source = PathfindSources.Classify(pair.Key);
            bySource[source] = bySource.TryGetValue(source, out var sum) ? sum.Plus(delta) : delta;
        }
        return new PathfindWindow(bySource, Frame >= before.Frame ? Frame - before.Frame : 0, Seconds - before.Seconds);
    }
}

/// <summary>締めた 1 区間の経路探しの結果（依頼元の分類ごと）。</summary>
public sealed class PathfindWindow
{
    public PathfindWindow(IReadOnlyDictionary<PathfindSource, PathfindCounter> bySource, uint simulationFrames, double seconds)
    {
        BySource = bySource;
        SimulationFrames = simulationFrames;
        Seconds = seconds;
        Total = bySource.Values.Aggregate(new PathfindCounter(0, 0, 0), (a, b) => a.Plus(b));
    }

    public IReadOnlyDictionary<PathfindSource, PathfindCounter> BySource { get; }

    public uint SimulationFrames { get; }

    public double Seconds { get; }

    public PathfindCounter Total { get; }

    public PathfindCounter Of(PathfindSource source) => BySource.TryGetValue(source, out var c) ? c : new PathfindCounter(0, 0, 0);

    /// <summary>シミュレーション 1 フレームあたりの件数。フレームが進んでいなければ NaN。</summary>
    public double PerFrame(long queries) => SimulationFrames == 0 ? double.NaN : (double)queries / SimulationFrames;

    /// <summary>経路探しの作業量全体に占める割合（0〜1）。作業が無ければ NaN。</summary>
    public double WorkShare(PathfindSource source) => Total.Work <= 0 ? double.NaN : Of(source).Work / Total.Work;

    /// <summary>行き先が見つかった割合（0〜1）。依頼が無ければ NaN。</summary>
    public double SuccessRate(PathfindSource source)
    {
        var c = Of(source);
        return c.Queries == 0 ? double.NaN : (double)c.Successes / c.Queries;
    }
}
