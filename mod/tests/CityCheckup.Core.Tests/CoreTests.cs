using System.Text.Json;

namespace CityCheckup.Core.Tests;

[TestClass]
public sealed class PathfindSourcesTests
{
    [TestMethod]
    [DataRow("FindJobSystem", PathfindSource.JobSearch)]
    [DataRow("ResourceBuyerSystem", PathfindSource.Shopping)]
    [DataRow("TaxiDispatchSystem", PathfindSource.ServiceDispatch)]
    [DataRow("PersonalCarAISystem", PathfindSource.VehicleRepath)]
    [DataRow("ResidentAISystem", PathfindSource.ResidentRepath)]
    [DataRow("LeisureSystem", PathfindSource.Trips)]
    [DataRow("SomethingNewSystem", PathfindSource.Other)]
    public void Classify_MapsKnownSystems(string system, PathfindSource expected)
        => Assert.AreEqual(expected, PathfindSources.Classify(system));

    [TestMethod]
    public void Snapshot_DiffGroupsSystemsAndWeighsByWork()
    {
        var before = new PathfindSnapshot(new Dictionary<string, PathfindCounter>
        {
            ["FindJobSystem"] = new(100, 20, 5.0),
            ["ResourceBuyerSystem"] = new(50, 50, 1.0),
        }, frame: 1000, seconds: 0);
        var after = new PathfindSnapshot(new Dictionary<string, PathfindCounter>
        {
            ["FindJobSystem"] = new(200, 40, 11.0),
            ["ResourceBuyerSystem"] = new(250, 250, 3.0),
            ["TaxiDispatchSystem"] = new(10, 10, 0.5),
        }, frame: 1100, seconds: 10);

        var w = after.Since(before);
        Assert.AreEqual(100u, w.SimulationFrames);
        Assert.AreEqual(310, w.Total.Queries);
        // 件数は買い物のほうが多いが、作業量は職探しが 6 / 8.5 を占める。
        Assert.AreEqual(6.0 / 8.5, w.WorkShare(PathfindSource.JobSearch), 1e-9);
        Assert.AreEqual(0.2, w.SuccessRate(PathfindSource.JobSearch), 1e-9);
        Assert.AreEqual(1.0, w.PerFrame(w.Of(PathfindSource.JobSearch).Queries), 1e-9);
        // 前回に無かったシステムは、今回の値がそのまま区間の値になる。
        Assert.AreEqual(10, w.Of(PathfindSource.ServiceDispatch).Queries);
    }

    [TestMethod]
    public void Snapshot_TreatsResetCountersAsFreshValues()
    {
        // 街を読み込み直すとゲームの累計は 0 に戻る。減っていたら、戻った後の値をそのまま使う。
        var before = new PathfindSnapshot(new Dictionary<string, PathfindCounter> { ["FindJobSystem"] = new(500, 10, 9.0) }, 10, 0);
        var after = new PathfindSnapshot(new Dictionary<string, PathfindCounter> { ["FindJobSystem"] = new(30, 3, 1.0) }, 20, 1);
        Assert.AreEqual(30, after.Since(before).Of(PathfindSource.JobSearch).Queries);
    }
}

[TestClass]
public sealed class SpeedSamplerTests
{
    [TestMethod]
    [DataRow(uint.MaxValue, 100u, 1.0)]
    [DataRow(200u, 100u, 1.0)]
    [DataRow(125u, 100u, 0.5)]
    [DataRow(100u, 100u, 0.0)]
    [DataRow(50u, 100u, 0.0)]
    public void PathfindSpeedFactor_MatchesGameThrottle(uint pending, uint frame, double expected)
        => Assert.AreEqual(expected, SpeedSampler.PathfindSpeedFactor(pending, frame), 1e-9);

    [TestMethod]
    public void Close_ReportsWaitShareBudgetAndIgnoresPause()
    {
        var s = new SpeedSampler();
        // 1 秒は速度を半分に絞られ、1 秒は全速。一時停止の 1 秒は数えない。
        s.Add(1.0, 1f, 0.5f, 0.5, 0.010, 4);
        s.Add(1.0, 1f, 1f, 1.0, 0.020, 0);
        s.Add(1.0, 0f, 0f, 0.0, 0.0, 100);
        var w = s.Close(3.0);
        Assert.AreEqual(2.0, w.RunningSeconds, 1e-9);
        Assert.AreEqual(0.25, w.PathfindWaitShare, 1e-9);
        Assert.AreEqual(15.0, w.SimFrameMilliseconds, 1e-9);
        Assert.AreEqual(1000.0 / 60, w.SimFrameBudgetMilliseconds, 1e-9);
        Assert.AreEqual(0.75, w.AverageActualSpeed, 1e-9);
        Assert.AreEqual(2.0, w.AveragePendingRequests, 1e-9);
        Assert.AreEqual(1.0, w.FramesPerSecond, 1e-9);
    }
}
[TestClass]
public sealed class JobMarketTests
{
    [TestMethod]
    public void ReachableVacancies_SumsLowerLevels()
    {
        var m = new JobMarket();
        m.AddVacanciesWithoutPosition(0, 3);
        m.AddVacanciesWithoutPosition(2, 5);
        CollectionAssert.AreEqual(new long[] { 3, 3, 8, 8, 8 }, m.ReachableVacancies());
    }

    [TestMethod]
    public void Hotspots_RanksByShortageAndMeasuresDistanceToJobs()
    {
        var m = new JobMarket();
        // 地区 (0,0) に失業者 3000 人、空き 0。隣の地区 (1,0) に空き 1 件、地区 (3,0)（6 km 先）に空き 2000。
        for (var i = 0; i < 3000; i++)
        {
            m.AddUnemployed(1, 100, 10, 100);
        }
        m.AddVacancies(1, 1, 2100, 10, 100);
        m.AddVacancies(1, 2000, 6100, 10, 100);
        // 地区 (0,5) に失業者 1500 人、同じ地区に空き 200。
        for (var i = 0; i < 1500; i++)
        {
            m.AddUnemployed(1, 100, 10, 10100);
        }
        m.AddVacancies(1, 200, 100, 10, 10100);

        var hotspots = m.Hotspots(5, 1000);
        Assert.AreEqual(2, hotspots.Count);
        Assert.AreEqual(3000, hotspots[0].Unemployed);
        // 隣の 1 件では不足分の半分（1500）に届かないので、6 km 先の地区までの距離になる。
        Assert.AreEqual(6000, hotspots[0].JobsReachMeters!.Value, 1e-6);
        Assert.AreEqual(100f, hotspots[0].X);
        // 2 番目の地区は同じ地区に空きがあるが、距離は「ほかの地区」の空きで測る。
        Assert.AreEqual(1300, hotspots[1].Unemployed - hotspots[1].Vacancies);
    }

    [TestMethod]
    public void Hotspots_ReachIsNullWhenCityHasTooFewVacancies()
    {
        var m = new JobMarket();
        for (var i = 0; i < 2000; i++)
        {
            m.AddUnemployed(0, 0, 0, 0);
        }
        m.AddVacancies(0, 100, 9000, 0, 0);
        Assert.IsNull(m.Hotspots(5, 1000).Single().JobsReachMeters);
    }

    [TestMethod]
    public void Hotspots_IgnoresSmallCells()
    {
        var m = new JobMarket();
        for (var i = 0; i < 999; i++)
        {
            m.AddUnemployed(0, 0, 0, 0);
        }
        Assert.AreEqual(0, m.Hotspots(5, 1000).Count);
    }
}

[TestClass]
public sealed class LoadReportTests
{
    [TestMethod]
    public void Findings_ExplainSlowdownHeaviestPathfindAndJobs()
    {
        var jobs = new JobMarket();
        for (var i = 0; i < 2000; i++)
        {
            jobs.AddUnemployed(0, 0, 0, 0);
        }
        jobs.AddVacancies(0, 100, 9000, 0, 0);
        var pathfind = new PathfindWindow(new Dictionary<PathfindSource, PathfindCounter>
        {
            [PathfindSource.JobSearch] = new(30, 6, 6.0),
            [PathfindSource.Shopping] = new(70, 70, 2.0),
        }, 10, 10);
        var speed = new SpeedSampler();
        speed.Add(10, 1f, 0.5f, 0.6, 0.030, 100);
        var report = new LoadReport
        {
            SelectedSpeed = 1,
            ActualSpeed = 0.5f,
            Jobs = jobs,
            Pathfind = pathfind,
            Speed = speed.Close(10),
        };

        var texts = report.Findings().Select(f => f.Text).ToList();
        Assert.AreEqual(6, texts.Count, string.Join("\n", texts));
        Assert.IsTrue(texts[0].Contains("×0.50"));
        // 経路探し待ちで 40% の時間速度を落とし、1 フレーム 30 ms は上限 16.7 ms を超えている。
        Assert.IsTrue(texts[1].Contains("40%"), texts[1]);
        Assert.IsTrue(texts[2].Contains("30.0 ms") && texts[2].Contains("16.7 ms"), texts[2]);
        // 件数では買い物が多いが、作業量では職探しが 75%、成功は 20%。
        Assert.IsTrue(texts[3].Contains("75%") && texts[3].Contains("職探し") && texts[3].Contains("20%"), texts[3]);
        Assert.IsTrue(texts[4].Contains("2,000"));
        Assert.IsTrue(texts[5].Contains("全部合わせても"));
    }
    [TestMethod]
    public void Findings_SaysNothingFoundForHealthyCity()
    {
        var report = new LoadReport { SelectedSpeed = 1, ActualSpeed = 1 };
        var only = report.Findings().Single();
        Assert.AreEqual(FindingLevel.Info, only.Level);
    }

    [TestMethod]
    public void ToJson_IsValidAndUsesNullForMissingParts()
    {
        var report = new LoadReport { SelectedSpeed = 3, ActualSpeed = float.NaN, ScanMilliseconds = 12.5 };
        using var doc = JsonDocument.Parse(report.ToJson(collecting: true));
        var root = doc.RootElement;
        Assert.IsTrue(root.GetProperty("collecting").GetBoolean());
        Assert.AreEqual(JsonValueKind.Null, root.GetProperty("pathfind").ValueKind);
        Assert.AreEqual(JsonValueKind.Null, root.GetProperty("jobs").ValueKind);
        // NaN は JSON に書けないので null になる。
        Assert.AreEqual(JsonValueKind.Null, root.GetProperty("speed").GetProperty("actual").ValueKind);
        Assert.AreEqual(3, root.GetProperty("speed").GetProperty("selected").GetDouble());
    }
}

[TestClass]
public sealed class JsonTextTests
{
    [TestMethod]
    public void EscapesControlCharactersAndKeepsJapanese()
    {
        var text = new JsonText().BeginObject().Key("a").Value("改行\n\"引用\"\u0001").EndObject().ToString();
        using var doc = JsonDocument.Parse(text);
        Assert.AreEqual("改行\n\"引用\"\u0001", doc.RootElement.GetProperty("a").GetString());
    }
}
