using System.Text.Json;

namespace SaveDataOptimizer.Core.Tests;

[TestClass]
public sealed class StrandedVehiclesTests
{
    [TestMethod]
    public void StuckTracker_CountsFramesOnlyWhileVehicleStaysStill()
    {
        var t = new StuckTracker();
        var id = StuckTracker.Id(5, 1);
        var next = new Dictionary<long, (float X, float Y, float Z, uint SinceFrame)>();
        Assert.AreEqual(0u, t.Observe(id, 0, 0, 0, 100, next));
        t.Commit(next);

        // 0.3 m しか動いていなければ「止まっている」として数え続ける。
        next = new();
        Assert.AreEqual(500u, t.Observe(id, 0.3f, 0, 0, 600, next));
        t.Commit(next);

        // 動いたら数え直し。
        next = new();
        Assert.AreEqual(0u, t.Observe(id, 10, 0, 0, 900, next));
    }

    [TestMethod]
    public void FlagTracker_ForgetsVehiclesThatStoppedBeingFlagged()
    {
        var t = new FlagTracker();
        var next = new Dictionary<long, uint>();
        t.Observe(1, 100, next);
        t.Commit(next);

        // 2 回目の探索で該当しなかった車両は忘れ、3 回目に再び該当したら数え直す。
        t.Commit(new Dictionary<long, uint>());
        next = new();
        Assert.AreEqual(0u, t.Observe(1, 9000, next));
        t.Commit(next);
        next = new();
        Assert.AreEqual(1000u, t.Observe(1, 10000, next));
    }

    [TestMethod]
    public void Groups_SelectByKeyAndSerialize()
    {
        var s = new StrandedVehicles { Scanned = 10, Pending = 2 };
        s.Add(StrandReason.OwnerGone, VehicleKind.Car, new StrandedVehicle(1, 1, 0, 0, 0, parked: true, hidden: false));
        s.Add(StrandReason.LaneGone, VehicleKind.Train, new StrandedVehicle(2, 1, 5, 6, 7, parked: false, hidden: false));
        s.Add(StrandReason.Stuck, VehicleKind.Car, new StrandedVehicle(3, 1, 0, 0, 0, parked: false, hidden: false));

        var chosen = s.InAll(new[] { "laneGone:train", "ownerGone:car", "laneGone:train" });
        CollectionAssert.AreEquivalent(new[] { 1, 2 }, chosen.Select(v => v.EntityIndex).ToArray());

        using var doc = JsonDocument.Parse(s.ToJson("ready", null));
        var groups = doc.RootElement.GetProperty("groups");
        // 理由の順（車線 → 持ち主 → 動いていない）に並ぶ。
        Assert.AreEqual("laneGone:train", groups[0].GetProperty("key").GetString());
        Assert.AreEqual("ownerGone", groups[1].GetProperty("reason").GetString());
        Assert.AreEqual(1, groups[1].GetProperty("parked").GetInt32());
        Assert.AreEqual(2, doc.RootElement.GetProperty("pending").GetInt32());
    }
}
