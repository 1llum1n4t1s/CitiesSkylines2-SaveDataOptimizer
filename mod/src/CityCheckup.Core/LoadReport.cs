using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

namespace CityCheckup.Core;

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

    /// <summary>ひとつの依頼元が経路探しの作業量のこの割合以上なら、主な負荷として知らせる。</summary>
    public const double PathfindWorkShareWarning = 0.25;

    /// <summary>経路探し待ちで速度を絞られた時間がこの割合以上なら、遅れの主な原因とみなす。</summary>
    public const double PathfindWaitWarning = 0.2;

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

    /// <summary>直近の区間の速度の内訳（経路探し待ち・処理時間）。まだ締まっていなければ null。</summary>
    public SpeedWindow? Speed { get; set; }

    /// <summary>職と失業者の集計。まだ数えていなければ null。</summary>
    public JobMarket? Jobs { get; set; }

    /// <summary>直近の集計（職・世帯）にかかった時間。mod 自身の負荷を隠さず見せるため。</summary>
    public double ScanMilliseconds { get; set; }

    /// <summary>所見の文の言語（ゲームの言語設定に合わせて mod 側が設定する）。</summary>
    public UiLanguage Language { get; set; } = UiLanguage.Japanese;

    private string L(string japanese, string english) => Language == UiLanguage.English ? english : japanese;

    public IReadOnlyList<Finding> Findings()
    {
        var list = new List<Finding>();
        // 遅れの判定は、その瞬間の値ではなく区間の平均で行う（一瞬の揺れで警告が出たり消えたりしないように）。
        var selected = Speed is { RunningSeconds: > 0 } sw ? sw.AverageSelectedSpeed : SelectedSpeed;
        var actual = Speed is { RunningSeconds: > 0 } sw2 ? sw2.AverageActualSpeed : ActualSpeed;
        if (selected > 0 && actual < selected * SlowSpeedRatio)
        {
            list.Add(new Finding(FindingLevel.Warning, string.Format(
                CultureInfo.InvariantCulture,
                L("設定速度 ×{0:0.#} に対して、実際の速度は ×{1:0.00} です。シミュレーションが追いついていません。", "Simulation speed is ×{1:0.00} against the ×{0:0.#} you set. The simulation cannot keep up."),
                selected, actual)));
            if (Speed is { } s)
            {
                // 遅れの原因を、ゲームが速度を落とす 2 つの仕組みに分けて伝える。
                if (s.PathfindWaitShare >= PathfindWaitWarning)
                {
                    list.Add(new Finding(FindingLevel.Warning, string.Format(
                        CultureInfo.InvariantCulture,
                        L("遅れの主な原因は経路探しの結果待ちです（時間の {0:0}% で速度を落としています）。下の「経路探しの作業量」が多い依頼を減らすと改善します。", "The main cause is waiting for pathfinding results (the game slowed down {0:0}% of the time). Reducing the requests with the most pathfinding work below will help."),
                        s.PathfindWaitShare * 100)));
                }
                if (s.SimFrameMilliseconds > s.SimFrameBudgetMilliseconds)
                {
                    list.Add(new Finding(FindingLevel.Warning, string.Format(
                        CultureInfo.InvariantCulture,
                        L("シミュレーション 1 フレームの処理に平均 {0:0.0} ms かかり、設定速度に必要な {1:0.0} ms を超えています。街の人や物の多さで処理が重くなっています。", "A simulation frame takes {0:0.0} ms on average, more than the {1:0.0} ms this speed needs. The number of people and objects in the city makes the simulation heavy."),
                        s.SimFrameMilliseconds, s.SimFrameBudgetMilliseconds)));
                }
            }
        }
        if (Pathfind is { } p && p.Total.Work > 0)
        {
            // 件数ではなく、ゲームが記録している探索の作業量で、いちばん重い依頼元を示す。
            var top = p.BySource.OrderByDescending(x => x.Value.Work).First().Key;
            var share = p.WorkShare(top);
            if (share >= PathfindWorkShareWarning)
            {
                var success = p.SuccessRate(top);
                var detail = top == PathfindSource.JobSearch
                    ? string.Format(CultureInfo.InvariantCulture, L("行き先が見つかったのは {0:0}% だけで、職に就けない人が繰り返し探しています。", "Only {0:0}% find a destination, so people who cannot get a job keep searching again."), success * 100)
                    : string.Format(CultureInfo.InvariantCulture, L("行き先が見つかったのは {0:0}% です。", "{0:0}% find a destination."), success * 100);
                list.Add(new Finding(FindingLevel.Warning, string.Format(
                    CultureInfo.InvariantCulture,
                    L("経路探しの作業量の {0:0}% は「{1}」です。{2}", "{0:0}% of pathfinding work is \"{1}\". {2}"),
                    share * 100, SourceLabel(top, Language), detail)));
            }
        }
        if (Jobs is { } jobs)
        {
            if (jobs.TotalUnemployed > jobs.TotalVacancies)
            {
                list.Add(new Finding(FindingLevel.Warning, string.Format(
                    CultureInfo.InvariantCulture,
                    L("職を探している人 {0:#,0} 人に対して、空き職場は {1:#,0} 件です。商業・工業・オフィスを増やすと職探しが減ります。", "{0:#,0} people are looking for work, but there are only {1:#,0} open jobs. More commercial, industrial and office zones will reduce job searching."),
                    jobs.TotalUnemployed, jobs.TotalVacancies)));
            }
            var far = jobs.Hotspots(1, HotspotMinUnemployed).FirstOrDefault(h => h.JobsReachMeters is null or >= FarJobsMeters);
            if (far is not null)
            {
                var distance = far.JobsReachMeters is { } m
                    ? string.Format(CultureInfo.InvariantCulture, L("足りない職の半分を埋める空きが約 {0:0.#} km 先まで行かないとありません", "enough open jobs to cover half of the shortage are about {0:0.#} km away"), m / 1000)
                    : L("市内の空きを全部合わせても足りない職を埋められません", "all open jobs in the city together cannot cover the shortage");
                list.Add(new Finding(FindingLevel.Warning, string.Format(
                    CultureInfo.InvariantCulture,
                    L("失業者 {0:#,0} 人の地区では、{1}。近くに職場を置くか、速い公共交通でつなぐと改善します。", "In a district with {0:#,0} unemployed people, {1}. Placing workplaces nearby or connecting it with fast public transport will help."),
                    far.Unemployed, distance)));
            }
        }
        if (list.Count == 0)
        {
            list.Add(new Finding(FindingLevel.Info, L("目立った負荷の原因は見つかっていません。", "No major cause of load was found.")));
        }
        return list;
    }

    public string ToJson(bool collecting)
    {
        var json = new JsonText().BeginObject();
        json.Key("collecting").Value(collecting);
        json.Key("speed").BeginObject()
            .Key("selected").Value(SelectedSpeed)
            .Key("actual").Value(ActualSpeed);
        json.Key("window");
        if (Speed is { } s)
        {
            json.BeginObject()
                .Key("pathfindWait").Value(s.PathfindWaitShare)
                .Key("simFrameMs").Value(s.SimFrameMilliseconds)
                .Key("budgetMs").Value(s.SimFrameBudgetMilliseconds)
                .Key("pending").Value(s.AveragePendingRequests)
                .Key("fps").Value(s.FramesPerSecond)
                .EndObject();
        }
        else
        {
            json.Value((string?)null);
        }
        json.EndObject();
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
                .Key("total").Value(p.Total.Queries)
                .Key("perFrame").Value(p.PerFrame(p.Total.Queries))
                .Key("sources").BeginArray();
            // 作業量の多い順に並べる（件数が少なくても 1 件が重い依頼を上に出す）。
            foreach (var pair in p.BySource.Where(c => c.Value.Queries > 0).OrderByDescending(c => c.Value.Work))
            {
                json.BeginObject()
                    .Key("key").Value(PathfindSources.Key(pair.Key))
                    .Key("count").Value(pair.Value.Queries)
                    .Key("perFrame").Value(p.PerFrame(pair.Value.Queries))
                    .Key("workShare").Value(p.WorkShare(pair.Key))
                    .Key("successRate").Value(p.SuccessRate(pair.Key))
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

    /// <summary>所見の文に使う依頼元の表示名（UI の SOURCE_LABELS と同じ言い方にそろえる）。</summary>
    public static string SourceLabel(PathfindSource source, UiLanguage language) => language == UiLanguage.English
        ? source switch
        {
            PathfindSource.JobSearch => "Job search",
            PathfindSource.Shopping => "Shopping",
            PathfindSource.ResidentRepath => "Pedestrian rerouting",
            PathfindSource.Trips => "Trips and leisure",
            PathfindSource.School => "School search",
            PathfindSource.Home => "Home search",
            PathfindSource.ServiceDispatch => "Service vehicle dispatch",
            PathfindSource.VehicleRepath => "Vehicle rerouting",
            _ => "Other",
        }
        : source switch
    {
        PathfindSource.JobSearch => "職探し",
        PathfindSource.Shopping => "市民の買い物",
        PathfindSource.ResidentRepath => "歩行者の経路の引き直し",
        PathfindSource.Trips => "外出・レジャー",
        PathfindSource.School => "学校探し",
        PathfindSource.Home => "住まい探し",
        PathfindSource.ServiceDispatch => "サービス車両の配車",
        PathfindSource.VehicleRepath => "車両の経路の引き直し",
        _ => "その他",
    };

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
