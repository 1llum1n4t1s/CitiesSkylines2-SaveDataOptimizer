using System;
using System.Collections.Generic;

namespace SaveDataOptimizer.Core;

/// <summary>経路探しの依頼元の分類。表示の並び順もこの順にする。</summary>
public enum PathfindSource
{
    JobSearch,
    JobSwitch,
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
/// </remarks>
public static class PathfindSources
{
    /// <param name="systemName">依頼を出したシステムの型名（例: FindJobSystem）。</param>
    /// <param name="employedSeeker">職探しの依頼で、探している人がすでに働いているか。職探し以外は null。</param>
    public static PathfindSource Classify(string systemName, bool? employedSeeker)
    {
        switch (systemName)
        {
            case "FindJobSystem":
                return employedSeeker == true ? PathfindSource.JobSwitch : PathfindSource.JobSearch;
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
        PathfindSource.JobSwitch => "jobSwitch",
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
/// 経路探しの依頼数を一定時間ずつ数え、直近の区間の結果を保持する。
/// </summary>
/// <remarks>
/// 数え始めから累計すると、パネルを開いた直後と長時間後で意味が変わってしまうため、
/// 一定の実時間ごとに区間を締め、直近で締めた区間の値を表示に使う。
/// </remarks>
public sealed class PathfindTally
{
    private readonly long[] m_Counts = new long[Enum.GetValues(typeof(PathfindSource)).Length];
    private uint m_StartFrame;
    private double m_StartSeconds;
    private bool m_Started;

    /// <summary>直近で締めた区間の結果。まだ 1 区間も締めていなければ null。</summary>
    public PathfindWindow? Last { get; private set; }

    public void Add(PathfindSource source) => m_Counts[(int)source]++;

    /// <summary>区間の開始を記録する（まだ始まっていない場合だけ）。</summary>
    public void Begin(uint frame, double seconds)
    {
        if (m_Started)
        {
            return;
        }
        m_Started = true;
        m_StartFrame = frame;
        m_StartSeconds = seconds;
        Array.Clear(m_Counts, 0, m_Counts.Length);
    }

    /// <summary>区間を締めて <see cref="Last"/> を更新し、次の区間を始める。</summary>
    public void Close(uint frame, double seconds)
    {
        if (!m_Started)
        {
            Begin(frame, seconds);
            return;
        }
        var counts = new Dictionary<PathfindSource, long>();
        for (var i = 0; i < m_Counts.Length; i++)
        {
            counts[(PathfindSource)i] = m_Counts[i];
        }
        Last = new PathfindWindow(counts, frame - m_StartFrame, seconds - m_StartSeconds);
        m_Started = false;
        Begin(frame, seconds);
    }

    /// <summary>パネルを閉じたときなど、数えるのをやめるときに途中の区間を捨てる。</summary>
    public void Reset()
    {
        m_Started = false;
        Last = null;
        Array.Clear(m_Counts, 0, m_Counts.Length);
    }
}

/// <summary>締めた 1 区間の依頼数。</summary>
public sealed class PathfindWindow
{
    public PathfindWindow(IReadOnlyDictionary<PathfindSource, long> counts, uint simulationFrames, double seconds)
    {
        Counts = counts;
        SimulationFrames = simulationFrames;
        Seconds = seconds;
        long total = 0;
        foreach (var n in counts.Values)
        {
            total += n;
        }
        Total = total;
    }

    public IReadOnlyDictionary<PathfindSource, long> Counts { get; }

    public uint SimulationFrames { get; }

    public double Seconds { get; }

    public long Total { get; }

    /// <summary>シミュレーション 1 フレームあたりの依頼数。フレームが進んでいなければ NaN。</summary>
    public double PerFrame(long count) => SimulationFrames == 0 ? double.NaN : (double)count / SimulationFrames;

    /// <summary>全体に占める割合（0〜1）。依頼が無ければ NaN。</summary>
    public double Share(long count) => Total == 0 ? double.NaN : (double)count / Total;
}
