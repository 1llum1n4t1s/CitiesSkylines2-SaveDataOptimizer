using System.Text.Json;

namespace CityCheckup.Core.Tests;

[TestClass]
public sealed class PathfindSourcesTests
{
    [TestMethod]
    [DataRow("FindJobSystem", false, PathfindSource.JobSearch)]
    [DataRow("FindJobSystem", true, PathfindSource.JobSwitch)]
    [DataRow("ResourceBuyerSystem", null, PathfindSource.Shopping)]
    [DataRow("TaxiDispatchSystem", null, PathfindSource.ServiceDispatch)]
    [DataRow("PersonalCarAISystem", null, PathfindSource.VehicleRepath)]
    [DataRow("ResidentAISystem", null, PathfindSource.ResidentRepath)]
    [DataRow("LeisureSystem", null, PathfindSource.Trips)]
    [DataRow("SomethingNewSystem", null, PathfindSource.Other)]
    public void Classify_MapsKnownSystems(string system, bool? employed, PathfindSource expected)
        => Assert.AreEqual(expected, PathfindSources.Classify(system, employed));

    [TestMethod]
    public void Tally_ClosesWindowWithRatesAndResetsCounts()
    {
        var tally = new PathfindTally();
        tally.Begin(100, 0);
        tally.Add(PathfindSource.JobSearch);
        tally.Add(PathfindSource.JobSearch);
        tally.Add(PathfindSource.Shopping);
        tally.Close(110, 10);

        var w = tally.Last!;
        Assert.AreEqual(3, w.Total);
        Assert.AreEqual(10u, w.SimulationFrames);
        Assert.AreEqual(0.2, w.PerFrame(w.Counts[PathfindSource.JobSearch]), 1e-9);
        Assert.AreEqual(2.0 / 3, w.Share(w.Counts[PathfindSource.JobSearch]), 1e-9);

        // 次の区間は 0 から数え直す。
        tally.Close(120, 20);
        Assert.AreEqual(0, tally.Last!.Total);
        Assert.IsTrue(double.IsNaN(tally.Last.Share(0)));
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
    public void Findings_ReportsSlowSpeedJobShortageAndFarJobs()
    {
        var jobs = new JobMarket();
        for (var i = 0; i < 2000; i++)
        {
            jobs.AddUnemployed(0, 0, 0, 0);
        }
        jobs.AddVacancies(0, 100, 9000, 0, 0);
        var counts = Enum.GetValues<PathfindSource>().ToDictionary(s => s, _ => 0L);
        counts[PathfindSource.JobSearch] = 30;
        counts[PathfindSource.Shopping] = 70;
        var report = new LoadReport
        {
            SelectedSpeed = 1,
            ActualSpeed = 0.5f,
            Jobs = jobs,
            Pathfind = new PathfindWindow(counts, 10, 10),
        };

        var texts = report.Findings().Select(f => f.Text).ToList();
        Assert.AreEqual(4, texts.Count, string.Join("\n", texts));
        Assert.IsTrue(texts[0].Contains("×0.50"));
        Assert.IsTrue(texts[1].Contains("30%"));
        Assert.IsTrue(texts[2].Contains("2,000"));
        Assert.IsTrue(texts[3].Contains("全部合わせても"));
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
