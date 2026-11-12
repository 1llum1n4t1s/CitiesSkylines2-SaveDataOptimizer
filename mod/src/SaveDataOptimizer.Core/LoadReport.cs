using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

namespace SaveDataOptimizer.Core;

/// <summary>
/// 負荷分析パネルに出す内容をまとめ、気づいた点（所見）を判定して JSON にする。
/// </summary>
/// <remarks>
/// 所見は「実測した値から言えること」だけにする。CPU 使用率のように mod で測っていない値は推測で書かない。
/// しきい値はゆろしてぃ（重い街）とミチャタウン（軽い街）の比較から決めた目安。
/// </remarks>
public sealed class LoadReport
{
    /// <summary>実際の速度が設定速度のこの割合を下回ったら「追いついていない」とみなす。</summary>
    public const double SlowSpeedRatio = 0.8;

    /// <summary>失業者の職探しが経路探しのこの割合以上なら、主な負荷として知らせる。</summary>
    public const double JobSearchShareWarning = 0.2;

    /// <summary>空き職場のある地区までこの距離（m）以上なら「職が遠い」とみなす。</summary>
    public const double FarJobsMeters = 4000;

    /// <summary>この人数以上の失業者がいる地区だけを「職の足りない地区」として扱う。</summary>
    public const long HotspotMinUnemployed = 1000;

    public float SelectedSpeed { get; set; }

    public float ActualSpeed { get; set; }

    public long Citizens { get; set; }

    public long HomelessHouseholds { get; set; }

    public long MovingAwayHouseholds { get; set; }

    /// <summary>直近の経路探しの集計。パネルを開いたばかりで区間が締まっていなければ null。</summary>
    public PathfindWindow? Pathfind { get; set; }

    /// <summary>職と失業者の集計。まだ数えていなければ null。</summary>
    public JobMarket? Jobs { get; set; }

    /// <summary>直近の集計（職・世帯）にかかった時間。mod 自身の負荷を隠さず見せるため。</summary>
    public double ScanMilliseconds { get; set; }

    public IReadOnlyList<Finding> Findings()
    {
        var list = new List<Finding>();
        if (SelectedSpeed > 0 && ActualSpeed < SelectedSpeed * SlowSpeedRatio)
        {
            list.Add(new Finding(FindingLevel.Warning, string.Format(
                CultureInfo.InvariantCulture,
                "設定速度 ×{0:0.#} に対して、実際の速度は ×{1:0.00} です。シミュレーションが追いついていません。",
                SelectedSpeed, ActualSpeed)));
        }
        if (Pathfind is { Total: > 0 } p)
        {
            var share = p.Share(p.Counts[PathfindSource.JobSearch]);
            if (share >= JobSearchShareWarning)
            {
                list.Add(new Finding(FindingLevel.Warning, string.Format(
                    CultureInfo.InvariantCulture,
                    "経路探しの {0:0}% が失業者の職探しです。職に就けない人が繰り返し探しています。",
                    share * 100)));
            }
        }
        if (Jobs is { } jobs)
        {
            if (jobs.TotalUnemployed > jobs.TotalVacancies)
            {
                list.Add(new Finding(FindingLevel.Warning, string.Format(
                    CultureInfo.InvariantCulture,
                    "職を探している人 {0:#,0} 人に対して、空き職場は {1:#,0} 件です。商業・工業・オフィスを増やすと職探しが減ります。",
                    jobs.TotalUnemployed, jobs.TotalVacancies)));
            }
            var far = jobs.Hotspots(1, HotspotMinUnemployed).FirstOrDefault(h => h.JobsReachMeters is null or >= FarJobsMeters);
            if (far is not null)
            {
                var distance = far.JobsReachMeters is { } m
                    ? string.Format(CultureInfo.InvariantCulture, "足りない職の半分を埋める空きが約 {0:0.#} km 先まで行かないとありません", m / 1000)
                    : "市内の空きを全部合わせても足りない職を埋められません";
                list.Add(new Finding(FindingLevel.Warning, string.Format(
                    CultureInfo.InvariantCulture,
                    "失業者 {0:#,0} 人の地区では、{1}。近くに職場を置くか、速い公共交通でつなぐと改善します。",
                    far.Unemployed, distance)));
            }
        }
        if (list.Count == 0)
        {
            list.Add(new Finding(FindingLevel.Info, "目立った負荷の原因は見つかっていません。"));
        }
        return list;
    }

    public string ToJson(bool collecting)
    {
        var json = new JsonText().BeginObject();
        json.Key("collecting").Value(collecting);
        json.Key("speed").BeginObject()
            .Key("selected").Value(SelectedSpeed)
            .Key("actual").Value(ActualSpeed)
            .EndObject();
        json.Key("citizens").Value(Citizens);
        json.Key("households").BeginObject()
            .Key("homeless").Value(HomelessHouseholds)
            .Key("movingAway").Value(MovingAwayHouseholds)
            .EndObject();
        json.Key("scanMs").Value(ScanMilliseconds);

        json.Key("pathfind");
        if (Pathfind is { } p)
        {
            json.BeginObject()
                .Key("seconds").Value(p.Seconds)
                .Key("frames").Value((long)p.SimulationFrames)
                .Key("total").Value(p.Total)
                .Key("perFrame").Value(p.PerFrame(p.Total))
                .Key("sources").BeginArray();
            foreach (var pair in p.Counts.Where(c => c.Value > 0).OrderByDescending(c => c.Value))
            {
                json.BeginObject()
                    .Key("key").Value(PathfindSources.Key(pair.Key))
                    .Key("count").Value(pair.Value)
                    .Key("perFrame").Value(p.PerFrame(pair.Value))
                    .Key("share").Value(p.Share(pair.Value))
                    .EndObject();
            }
            json.EndArray().EndObject();
        }
        else
        {
            json.Value((string?)null);
        }

        json.Key("jobs");
        if (Jobs is { } jobs)
        {
            json.BeginObject();
            WriteArray(json.Key("unemployed"), jobs.Unemployed);
            WriteArray(json.Key("vacancies"), jobs.Vacancies);
            WriteArray(json.Key("reachable"), jobs.ReachableVacancies());
            json.Key("commuters").Value(jobs.UnemployedCommuters);
            json.Key("hotspots").BeginArray();
            foreach (var h in jobs.Hotspots(5, HotspotMinUnemployed))
            {
                json.BeginObject()
                    .Key("unemployed").Value(h.Unemployed)
                    .Key("vacancies").Value(h.Vacancies)
                    .Key("reachKm").Value(h.JobsReachMeters is { } m ? m / 1000 : double.NaN)
                    .Key("x").Value(h.X)
                    .Key("y").Value(h.Y)
                    .Key("z").Value(h.Z)
                    .EndObject();
            }
            json.EndArray().EndObject();
        }
        else
        {
            json.Value((string?)null);
        }

        json.Key("findings").BeginArray();
        foreach (var f in Findings())
        {
            json.BeginObject()
                .Key("level").Value(f.Level == FindingLevel.Warning ? "warning" : "info")
                .Key("text").Value(f.Text)
                .EndObject();
        }
        json.EndArray();
        return json.EndObject().ToString();
    }

    private static void WriteArray(JsonText json, IEnumerable<long> values)
    {
        json.BeginArray();
        foreach (var v in values)
        {
            json.Value(v);
        }
        json.EndArray();
    }
}

public enum FindingLevel
{
    Info,
    Warning,
}

public sealed class Finding
{
    public Finding(FindingLevel level, string text)
    {
        Level = level;
        Text = text ?? throw new ArgumentNullException(nameof(text));
    }

    public FindingLevel Level { get; }

    public string Text { get; }
}
