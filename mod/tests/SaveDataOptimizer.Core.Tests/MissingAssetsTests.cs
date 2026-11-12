using System.Text.Json;

namespace SaveDataOptimizer.Core.Tests;

[TestClass]
public sealed class MissingAssetsTests
{
    private static MissingAssets Sample()
    {
        var m = new MissingAssets();
        m.Add("StaticObjectPrefab", "Bridge", MissingKind.Object, new MissingInstance(10, 1, 1, 2, 3, deletable: true));
        m.Add("StaticObjectPrefab", "Bridge", MissingKind.Object, new MissingInstance(11, 1, 4, 5, 6, deletable: false));
        m.Add("StaticObjectPrefab", "Bridge", MissingKind.Object, new MissingInstance(12, 2, 7, 8, 9, deletable: true));
        m.Add("RoadPrefab", "Road", MissingKind.Net, new MissingInstance(20, 1, 0, 0, 0, deletable: false));
        return m;
    }

    [TestMethod]
    public void Groups_AreOrderedByCountAndCountDeletable()
    {
        var groups = Sample().Groups.ToList();
        Assert.AreEqual("Bridge", groups[0].PrefabName);
        Assert.AreEqual(3, groups[0].Instances.Count);
        Assert.AreEqual(2, groups[0].DeletableCount);
        Assert.AreEqual(0, groups[1].DeletableCount);
    }

    [TestMethod]
    public void DeletableIn_ReturnsOnlyDeletableInstancesOfChosenGroups()
    {
        var m = Sample();
        var targets = m.DeletableIn(new[] { "StaticObjectPrefab:Bridge", "RoadPrefab:Road", "StaticObjectPrefab:Bridge", "Unknown:X" });
        CollectionAssert.AreEqual(new[] { 10, 12 }, targets.Select(t => t.EntityIndex).ToArray());
    }

    [TestMethod]
    public void ToJson_ListsGroupsWithPositions()
    {
        using var doc = JsonDocument.Parse(Sample().ToJson("ready", 5));
        var root = doc.RootElement;
        Assert.AreEqual("ready", root.GetProperty("state").GetString());
        Assert.AreEqual(4, root.GetProperty("total").GetInt32());
        Assert.AreEqual(5, root.GetProperty("lastDeleted").GetInt32());
        var first = root.GetProperty("groups")[0];
        Assert.AreEqual("StaticObjectPrefab:Bridge", first.GetProperty("key").GetString());
        Assert.AreEqual("object", first.GetProperty("kind").GetString());
        Assert.AreEqual(7, first.GetProperty("positions")[2][0].GetDouble());
    }
}
