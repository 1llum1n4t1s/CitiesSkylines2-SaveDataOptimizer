using System;
using System.Collections.Generic;
using System.Linq;

namespace CityCheckup.Core;

/// <summary>欠落したアセットが置かれていた物の種類。</summary>
public enum MissingKind
{
    /// <summary>建物（成長型・サービス施設など）。</summary>
    Building,

    /// <summary>建物以外の物（小物・木・歩道橋のような置き物）。</summary>
    Object,

    /// <summary>道路・線路などのネットワーク。</summary>
    Net,

    /// <summary>区画・地表などのエリア。</summary>
    Area,
}

/// <summary>欠落したアセットの置かれた 1 か所。</summary>
public readonly struct MissingInstance
{
    public MissingInstance(int entityIndex, int entityVersion, float x, float y, float z, bool deletable)
    {
        EntityIndex = entityIndex;
        EntityVersion = entityVersion;
        X = x;
        Y = y;
        Z = z;
        Deletable = deletable;
    }

    /// <summary>ゲームの Entity の Index と Version。Core はゲームの型を持たないので数値で預かる。</summary>
    public int EntityIndex { get; }

    public int EntityVersion { get; }

    public float X { get; }

    public float Y { get; }

    public float Z { get; }

    /// <summary>この mod から単独で消してよいか（ゲームの「削除」ボタンと同じ考え方で判定した結果）。</summary>
    public bool Deletable { get; }
}

/// <summary>同じ欠落アセット（元の型名と名前が同じもの）の置き場所のまとまり。</summary>
public sealed class MissingGroup
{
    private readonly List<MissingInstance> m_Instances = new();

    public MissingGroup(string key, string prefabType, string prefabName, MissingKind kind)
    {
        Key = key;
        PrefabType = prefabType;
        PrefabName = prefabName;
        Kind = kind;
    }

    /// <summary>UI との受け渡しに使う識別子（元の型名と名前から作る）。</summary>
    public string Key { get; }

    /// <summary>元のプレハブの型名（例: StaticObjectPrefab）。</summary>
    public string PrefabType { get; }

    /// <summary>元のプレハブの名前。mod のアセット名がそのまま残っている。</summary>
    public string PrefabName { get; }

    public MissingKind Kind { get; }

    public IReadOnlyList<MissingInstance> Instances => m_Instances;

    public int DeletableCount => m_Instances.Count(i => i.Deletable);

    public void Add(MissingInstance instance) => m_Instances.Add(instance);
}

/// <summary>
/// アンインストールした mod のアセット（欠落したプレハブ）を参照している物の一覧。
/// </summary>
/// <remarks>
/// ゲームは読み込み時に見つからないプレハブを消さず、「欠落」扱いのプレハブ（PrefabData が無効）に差し替えて
/// 代わりの見た目で表示し続け、保存するとそのまま残る。ここではそれを元のアセット名ごとにまとめる。
/// 一覧の作成（ゲームの探索）は mod 側、まとめ方と表示データは Core 側が受け持つ。
/// </remarks>
public sealed class MissingAssets
{
    // 1 グループあたり UI へ渡す位置の数。「見に行く」で順に回るのに使う。全件だと JSON が大きくなりすぎる。
    public const int PositionsPerGroup = 200;

    private readonly Dictionary<string, MissingGroup> m_Groups = new(StringComparer.Ordinal);

    public static string MakeKey(string prefabType, string prefabName) => prefabType + ":" + prefabName;

    public IEnumerable<MissingGroup> Groups => m_Groups.Values
        .OrderByDescending(g => g.Instances.Count)
        .ThenBy(g => g.PrefabName, StringComparer.Ordinal);

    public int TotalInstances => m_Groups.Values.Sum(g => g.Instances.Count);

    public void Add(string prefabType, string prefabName, MissingKind kind, MissingInstance instance)
    {
        var key = MakeKey(prefabType, prefabName);
        if (!m_Groups.TryGetValue(key, out var group))
        {
            group = new MissingGroup(key, prefabType, prefabName, kind);
            m_Groups[key] = group;
        }
        group.Add(instance);
    }

    public MissingGroup? Find(string key) => m_Groups.TryGetValue(key, out var g) ? g : null;

    /// <summary>選ばれたグループのうち、消してよい置き場所だけを返す。</summary>
    public IReadOnlyList<MissingInstance> DeletableIn(IEnumerable<string> keys)
    {
        var result = new List<MissingInstance>();
        foreach (var key in keys.Distinct(StringComparer.Ordinal))
        {
            if (m_Groups.TryGetValue(key, out var g))
            {
                result.AddRange(g.Instances.Where(i => i.Deletable));
            }
        }
        return result;
    }

    public static string KindKey(MissingKind kind) => kind switch
    {
        MissingKind.Building => "building",
        MissingKind.Object => "object",
        MissingKind.Net => "net",
        _ => "area",
    };

    /// <param name="state">UI に見せる状態（idle / scanning / ready / deleting）。</param>
    /// <param name="lastDeleted">直前の削除で消した数（まだ削除していなければ null）。</param>
    public string ToJson(string state, int? lastDeleted)
    {
        var json = new JsonText().BeginObject();
        json.Key("state").Value(state);
        json.Key("total").Value(TotalInstances);
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
                .Key("key").Value(g.Key)
                .Key("type").Value(g.PrefabType)
                .Key("name").Value(g.PrefabName)
                .Key("kind").Value(KindKey(g.Kind))
                .Key("count").Value(g.Instances.Count)
                .Key("deletable").Value(g.DeletableCount)
                .Key("positions").BeginArray();
            foreach (var i in g.Instances.Take(PositionsPerGroup))
            {
                json.BeginArray().Value(i.X).Value(i.Y).Value(i.Z).EndArray();
            }
            json.EndArray().EndObject();
        }
        json.EndArray();
        return json.EndObject().ToString();
    }
}
