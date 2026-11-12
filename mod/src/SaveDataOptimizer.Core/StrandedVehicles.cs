using System;
using System.Collections.Generic;
using System.Linq;

namespace SaveDataOptimizer.Core;

/// <summary>取り残された車両と判断した理由。表示の並び順もこの順。</summary>
public enum StrandReason
{
    /// <summary>今いる車線（道路・線路・駐車場の区画）が無くなっている、または削除済み。</summary>
    LaneGone,

    /// <summary>車線は残っているが、その車線の「上にいる物」の一覧に入っていない（交通から外れて置き去りの状態）。</summary>
    NotOnLane,

    /// <summary>持ち主（車なら世帯や運転していた人、業務車両なら車庫の建物）がもういない。</summary>
    OwnerGone,

    /// <summary>公共交通の車両なのに、どの路線にも組み込まれておらず、車庫へ帰る途中でもない。</summary>
    NoRoute,

    /// <summary>走行中のはずなのに、しばらく全く動いていない（渋滞や停車中も含むので参考扱い）。</summary>
    Stuck,
}

/// <summary>車両の種類。</summary>
public enum VehicleKind
{
    Car,
    Train,
    Other,
}

/// <summary>取り残された車両 1 台（列車は編成 1 本）。</summary>
public readonly struct StrandedVehicle
{
    public StrandedVehicle(int entityIndex, int entityVersion, float x, float y, float z, bool parked, bool hidden)
    {
        EntityIndex = entityIndex;
        EntityVersion = entityVersion;
        X = x;
        Y = y;
        Z = z;
        Parked = parked;
        Hidden = hidden;
    }

    public int EntityIndex { get; }

    public int EntityVersion { get; }

    public float X { get; }

    public float Y { get; }

    public float Z { get; }

    /// <summary>駐車（駐留）中か。</summary>
    public bool Parked { get; }

    /// <summary>画面に出ていない（ゲームが隠している）車両か。</summary>
    public bool Hidden { get; }
}

/// <summary>
/// 線路や駐車場を撤去したあとに残り続ける列車・車を、理由と種類ごとにまとめる。
/// </summary>
/// <remarks>
/// 探索（ゲームの車両をたどる）は mod 側、まとめ方・表示データ・「動いていない」の判定は Core 側が受け持つ。
/// 「動いていない」は 1 回の探索では分からないので、前回の位置と比べて判定する（<see cref="StuckTracker"/>）。
/// </remarks>
public sealed class StrandedVehicles
{
    public const int PositionsPerGroup = 200;

    private readonly Dictionary<(StrandReason, VehicleKind), List<StrandedVehicle>> m_Groups = new();

    /// <summary>探索した車両の総数（列車は編成ごと）。判断の目安として表示する。</summary>
    public long Scanned { get; set; }

    /// <summary>取り残しの疑いがあるが、まだ続いている時間が短く確かめ中の台数。</summary>
    public long Pending { get; set; }

    public static string Key(StrandReason reason, VehicleKind kind) => $"{ReasonKey(reason)}:{KindKey(kind)}";

    public static string ReasonKey(StrandReason reason) => reason switch
    {
        StrandReason.LaneGone => "laneGone",
        StrandReason.NotOnLane => "notOnLane",
        StrandReason.OwnerGone => "ownerGone",
        StrandReason.NoRoute => "noRoute",
        _ => "stuck",
    };

    public static string KindKey(VehicleKind kind) => kind switch
    {
        VehicleKind.Car => "car",
        VehicleKind.Train => "train",
        _ => "other",
    };

    public void Add(StrandReason reason, VehicleKind kind, StrandedVehicle vehicle)
    {
        if (!m_Groups.TryGetValue((reason, kind), out var list))
        {
            list = new List<StrandedVehicle>();
            m_Groups[(reason, kind)] = list;
        }
        list.Add(vehicle);
    }

    public int Total => m_Groups.Values.Sum(l => l.Count);

    public IEnumerable<(StrandReason Reason, VehicleKind Kind, IReadOnlyList<StrandedVehicle> Vehicles)> Groups => m_Groups
        .OrderBy(p => p.Key.Item1)
        .ThenBy(p => p.Key.Item2)
        .Select(p => (p.Key.Item1, p.Key.Item2, (IReadOnlyList<StrandedVehicle>)p.Value));

    public IReadOnlyList<StrandedVehicle> In(string key)
    {
        foreach (var g in Groups)
        {
            if (Key(g.Reason, g.Kind) == key)
            {
                return g.Vehicles;
            }
        }
        return Array.Empty<StrandedVehicle>();
    }

    public IReadOnlyList<StrandedVehicle> InAll(IEnumerable<string> keys) =>
        keys.Distinct(StringComparer.Ordinal).SelectMany(In).ToList();

    public string ToJson(string state, int? lastDeleted)
    {
        var json = new JsonText().BeginObject();
        json.Key("state").Value(state);
        json.Key("scanned").Value(Scanned);
        json.Key("total").Value(Total);
        json.Key("pending").Value(Pending);
        json.Key("lastDeleted");
        if (lastDeleted is { } n)
        {
            json.Value(n);
        }
        else
        {
            json.Value((string?)null);
        }
        json.Key("groups").BeginArray();
        foreach (var g in Groups)
        {
            json.BeginObject()
                .Key("key").Value(Key(g.Reason, g.Kind))
                .Key("reason").Value(ReasonKey(g.Reason))
                .Key("kind").Value(KindKey(g.Kind))
                .Key("count").Value(g.Vehicles.Count)
                .Key("parked").Value(g.Vehicles.Count(v => v.Parked))
                .Key("hidden").Value(g.Vehicles.Count(v => v.Hidden))
                .Key("positions").BeginArray();
            foreach (var v in g.Vehicles.Take(PositionsPerGroup))
            {
                json.BeginArray().Value(v.X).Value(v.Y).Value(v.Z).EndArray();
            }
            json.EndArray().EndObject();
        }
        json.EndArray();
        return json.EndObject().ToString();
    }
}

/// <summary>
/// 走行中の車両の位置を探索ごとに覚え、一定時間以上ほとんど動いていない車両を見つける。
/// </summary>
/// <remarks>
/// 渋滞・信号待ち・駅での停車でも止まるため、ここで見つかるのは「取り残しの疑い」まで。
/// 時間はシミュレーションのフレーム数で測る（一時停止中は進まないので、止めている間に疑いが増えない）。
/// </remarks>
public sealed class StuckTracker
{
    /// <summary>この距離（m）以内しか動いていなければ「動いていない」とみなす。</summary>
    public const float StillMeters = 0.5f;

    /// <summary>このフレーム数（ゲーム内でおよそ 2 分。1 フレーム = 1/60 秒相当）以上動いていなければ疑う。</summary>
    public const uint StuckFrames = 60 * 120;

    private Dictionary<long, (float X, float Y, float Z, uint SinceFrame)> m_Last = new();

    public static long Id(int index, int version) => ((long)index << 32) | (uint)version;

    /// <summary>
    /// 今回の位置を記録し、前回から動いていない車両が止まっていたフレーム数を返す（初めて見た車両・動いた車両は 0）。
    /// </summary>
    public uint Observe(long id, float x, float y, float z, uint frame, Dictionary<long, (float X, float Y, float Z, uint SinceFrame)> next)
    {
        var since = frame;
        if (m_Last.TryGetValue(id, out var last))
        {
            var dx = x - last.X;
            var dy = y - last.Y;
            var dz = z - last.Z;
            if (dx * dx + dy * dy + dz * dz <= StillMeters * StillMeters)
            {
                since = last.SinceFrame;
                // 動いていない間は最初に止まっていた位置を基準にし続ける（少しずつずれても積み上がらないように）。
                x = last.X;
                y = last.Y;
                z = last.Z;
            }
        }
        next[id] = (x, y, z, since);
        return frame - since;
    }

    /// <summary>今回の探索で見た車両だけを次回の比較対象にする（消えた車両を覚え続けない）。</summary>
    public void Commit(Dictionary<long, (float X, float Y, float Z, uint SinceFrame)> next) => m_Last = next;

    public void Reset() => m_Last = new();
}

/// <summary>
/// 「持ち主がいない」「路線に属さない」と判定された車両が、その状態のまま続いているかを探索ごとに追う。
/// </summary>
/// <remarks>
/// 世帯が空になった直後や、運転者が亡くなった直後の車は、ゲームの片付け待ちで一時的にこの状態になる（ゆろしてぃで確認）。
/// 一時的なものを消さないよう、<see cref="StuckTracker.StuckFrames"/> 以上続いたものだけを取り残しとして扱う。
/// </remarks>
public sealed class FlagTracker
{
    private Dictionary<long, uint> m_First = new();

    /// <summary>今回も該当した車両を記録し、最初に該当したときからのフレーム数を返す。</summary>
    public uint Observe(long id, uint frame, Dictionary<long, uint> next)
    {
        var first = m_First.TryGetValue(id, out var f) ? f : frame;
        next[id] = first;
        return frame - first;
    }

    /// <summary>今回該当した車両だけを次回へ引き継ぐ（該当しなくなった車両は数え直し）。</summary>
    public void Commit(Dictionary<long, uint> next) => m_First = next;

    public void Reset() => m_First = new();
}
