using System;
using System.Collections.Generic;
using System.Linq;
using Colossal.Serialization.Entities;
using Colossal.UI.Binding;
using Game;
using Game.Areas;
using Game.Buildings;
using Game.Common;
using Game.Net;
using Game.Prefabs;
using Game.Rendering;
using Game.Tools;
using Game.UI;
using Game.UI.InGame;
using CityCheckup.Core;
using CityCheckup.Ui;
using Unity.Collections;
using Unity.Entities;
using Unity.Mathematics;
using Debug = UnityEngine.Debug;
using Transform = Game.Objects.Transform;

namespace CityCheckup.Missing;

/// <summary>
/// アンインストールした mod のアセット（欠落したプレハブ）を参照している物を探し、選ばれたものを消す。
/// </summary>
/// <remarks>
/// 見つけ方: ゲームは読み込み時に見つからないプレハブを「欠落」扱いにし、そのプレハブの PrefabData を無効にする
/// （ResolvePrefabsSystem / InitializeObsoleteSystem）。元の名前は PrefabSystem.GetObsoleteID で引ける。
/// そこで PrefabData が無効なプレハブを集め、それを PrefabRef で参照している物を数える。
///
/// 消し方: ゲームの情報パネルの「削除」ボタン（ActionsSection.OnDelete）と同じく、EndFrameBarrier で Deleted を付ける。
/// 付属物（Owner でつながる物）の後始末はゲームの通常の削除処理に任せる。
/// 単独で消してよいかの判定も同じボタンの条件に合わせる（置き物は自前の持ち主がいない静的な物、建物は付属建物でない物）。
/// ただし成長型の建物は、欠落していれば作り直されないため、ここでは消してよいものとして扱う。
/// 道路・線路とエリアはゲームのツールを通した削除が必要なので、一覧と「見に行く」だけにする。
///
/// 探索は全ての物をたどるので重い。UI の「探す」ボタン（または削除の後）でだけ動かす。
/// </remarks>
public sealed partial class MissingAssetSystem : UISystemBase
{
    private const string Group = PanelUISystem.Group;

    private PrefabSystem m_Prefabs = null!;
    private CameraUpdateSystem m_Camera = null!;
    private SelectedInfoUISystem m_SelectedInfo = null!;
    private EndFrameBarrier m_EndFrameBarrier = null!;
    private EntityQuery m_ObsoletePrefabs;
    private EntityQuery m_Placed;
    private ValueBinding<string> m_Binding = null!;
    private ValueBinding<bool> m_HasSelection = null!;

    private MissingAssets m_Result = new();
    private string m_State = "idle";
    private int? m_LastDeleted;
    private bool m_InGame;

    // 削除を頼んでから探し直すまでのフレーム数。EndFrameBarrier の反映と、付属物の後始末を待つ。
    private const int RescanDelayFrames = 10;
    private int m_RescanCountdown;

    // 動作確認用（DebugMarkers）の進行。
    private int m_DebugStep;
    private float m_DebugAt;

    public override GameMode gameMode => GameMode.Game;

    protected override void OnCreate()
    {
        base.OnCreate();
        m_Prefabs = World.GetOrCreateSystemManaged<PrefabSystem>();
        m_Camera = World.GetOrCreateSystemManaged<CameraUpdateSystem>();
        m_SelectedInfo = World.GetOrCreateSystemManaged<SelectedInfoUISystem>();
        m_EndFrameBarrier = World.GetOrCreateSystemManaged<EndFrameBarrier>();

        // InitializeObsoleteSystem が欠落プレハブを探すときと同じ条件。
        m_ObsoletePrefabs = GetEntityQuery(new EntityQueryDesc
        {
            Any = new[]
            {
                ComponentType.ReadOnly<ObjectData>(),
                ComponentType.ReadOnly<NetData>(),
                ComponentType.ReadOnly<AggregateNetData>(),
                ComponentType.ReadOnly<NetLaneArchetypeData>(),
                ComponentType.ReadOnly<AreaData>(),
            },
            Disabled = new[] { ComponentType.ReadOnly<PrefabData>() },
        });

        // 街に置かれた物（物・建物・道路の区間・エリア）。道路のノードは区間と重複するので数えない。
        m_Placed = GetEntityQuery(new EntityQueryDesc
        {
            All = new[] { ComponentType.ReadOnly<PrefabRef>() },
            Any = new[]
            {
                ComponentType.ReadOnly<Game.Objects.Object>(),
                ComponentType.ReadOnly<Edge>(),
                ComponentType.ReadOnly<Area>(),
            },
            None = new[] { ComponentType.ReadOnly<Temp>(), ComponentType.ReadOnly<Deleted>() },
        });

        m_Binding = new ValueBinding<string>(Group, "missing", m_Result.ToJson(m_State, null));
        AddBinding(m_Binding);
        AddBinding(new TriggerBinding(Group, "scanMissing", Scan));
        AddBinding(new TriggerBinding<string, int>(Group, "jumpMissing", OnJump));
        AddBinding(new TriggerBinding<string>(Group, "deleteMissing", OnDelete));

        // ゲームの情報パネル（何かを選択したときに左上に出る）と mod のパネルが重なるため、
        // 選択中かどうかを UI に知らせ、選択中は mod のパネルを畳む。ボタンを押すと選択を解いて開き直す。
        m_HasSelection = new ValueBinding<bool>(Group, "hasSelection", false);
        AddBinding(m_HasSelection);
        AddBinding(new TriggerBinding(Group, "clearSelection", () => m_SelectedInfo.SetSelection(Entity.Null)));
    }

    protected override void OnGamePreload(Purpose purpose, GameMode mode)
    {
        base.OnGamePreload(purpose, mode);
        m_InGame = false;
        // 別の街を読み込んだら前の一覧は無効（Entity が別物になる）。
        m_Result = new MissingAssets();
        m_State = "idle";
        m_LastDeleted = null;
        m_Binding.Update(m_Result.ToJson(m_State, m_LastDeleted));
    }

    protected override void OnGameLoadingComplete(Purpose purpose, GameMode mode)
    {
        base.OnGameLoadingComplete(purpose, mode);
        m_InGame = mode == GameMode.Game;
        m_DebugStep = 0;
        m_DebugAt = UnityEngine.Time.realtimeSinceStartup + 5f;
    }

    protected override void OnUpdate()
    {
        base.OnUpdate();
        if (!m_InGame)
        {
            return;
        }
        // 値が変わったときだけ UI へ送られる（ValueBinding が前回と比べる）。
        m_HasSelection.Update(m_SelectedInfo.selectedEntity != Entity.Null);
        if (m_RescanCountdown > 0 && --m_RescanCountdown == 0)
        {
            Scan();
        }
        RunDebug();
    }

    private void Scan()
    {
        var result = new MissingAssets();
        var obsolete = new Dictionary<Entity, (string Type, string Name)>();
        using (var prefabs = m_ObsoletePrefabs.ToEntityArray(Allocator.Temp))
        {
            foreach (var prefab in prefabs)
            {
                try
                {
                    var id = m_Prefabs.GetObsoleteID(prefab);
                    obsolete[prefab] = (TypeOf(id.ToString()), id.GetName());
                }
                catch (Exception e)
                {
                    // 欠落の記録と食い違う場合は、その種類だけ名前を伏せて数える（一覧全体は止めない）。
                    obsolete[prefab] = ("?", "(名前不明)");
                    Debug.LogWarning($"{Mod.LogPrefix} 欠落プレハブの名前を引けませんでした: {e.Message}");
                }
            }
        }

        if (obsolete.Count > 0)
        {
            var em = EntityManager;
            using var entities = m_Placed.ToEntityArray(Allocator.Temp);
            using var refs = m_Placed.ToComponentDataArray<PrefabRef>(Allocator.Temp);
            for (var i = 0; i < entities.Length; i++)
            {
                if (!obsolete.TryGetValue(refs[i].m_Prefab, out var id))
                {
                    continue;
                }
                var entity = entities[i];
                // 持ち主も欠落アセットなら、持ち主の一部として扱い一覧には出さない（持ち主を消せば一緒に消える）。
                if (em.HasComponent<Owner>(entity))
                {
                    var owner = em.GetComponentData<Owner>(entity).m_Owner;
                    if (em.Exists(owner) && em.HasComponent<PrefabRef>(owner) && obsolete.ContainsKey(em.GetComponentData<PrefabRef>(owner).m_Prefab))
                    {
                        continue;
                    }
                }
                var kind = KindOf(em, entity);
                var p = PositionOf(em, entity);
                result.Add(id.Type, id.Name, kind, new MissingInstance(entity.Index, entity.Version, p.x, p.y, p.z, IsDeletable(em, entity, kind)));
            }
        }

        m_Result = result;
        m_State = "ready";
        m_Binding.Update(m_Result.ToJson(m_State, m_LastDeleted));
        Debug.Log($"{Mod.LogPrefix} missing scan: {obsolete.Count} obsolete prefabs, {result.TotalInstances} placed");
    }

    private void OnJump(string key, int index)
    {
        var group = m_Result.Find(key);
        if (group is null || group.Instances.Count == 0)
        {
            return;
        }
        var i = group.Instances[((index % group.Instances.Count) + group.Instances.Count) % group.Instances.Count];
        CameraJump.To(m_Camera, i.X, i.Y, i.Z);
        // ゲームの選択状態にして、輪郭の強調と情報パネルを出す（クリックで選んだのと同じ）。
        var entity = new Entity { Index = i.EntityIndex, Version = i.EntityVersion };
        if (EntityManager.Exists(entity))
        {
            m_SelectedInfo.SetSelection(entity);
        }
    }

    /// <param name="keys">消すグループの Key を改行でつないだもの。</param>
    private void OnDelete(string keys)
    {
        var targets = m_Result.DeletableIn(keys.Split(new[] { '\n' }, StringSplitOptions.RemoveEmptyEntries));
        var em = EntityManager;
        var buffer = m_EndFrameBarrier.CreateCommandBuffer();
        var count = 0;
        foreach (var t in targets)
        {
            var entity = new Entity { Index = t.EntityIndex, Version = t.EntityVersion };
            // 一覧を作った後に消えたり別の物に変わったりしていないか、消す直前に確かめ直す。
            if (!em.Exists(entity) || em.HasComponent<Deleted>(entity) || !em.HasComponent<PrefabRef>(entity))
            {
                continue;
            }
            var prefab = em.GetComponentData<PrefabRef>(entity).m_Prefab;
            if (!em.Exists(prefab) || !em.HasComponent<PrefabData>(prefab) || em.IsComponentEnabled<PrefabData>(prefab))
            {
                continue;
            }
            buffer.AddComponent<Deleted>(entity);
            count++;
        }
        m_LastDeleted = count;
        m_State = "deleting";
        m_Binding.Update(m_Result.ToJson(m_State, m_LastDeleted));
        m_RescanCountdown = RescanDelayFrames;
        Debug.Log($"{Mod.LogPrefix} missing delete: {count} entities");
    }

    private static string TypeOf(string id)
    {
        // PrefabID.ToString() は「型:名前 (ハッシュ)」の形。型の部分だけ取り出す。
        var colon = id.IndexOf(':');
        return colon > 0 ? id.Substring(0, colon) : id;
    }

    private static MissingKind KindOf(EntityManager em, Entity e)
    {
        if (em.HasComponent<Building>(e))
        {
            return MissingKind.Building;
        }
        if (em.HasComponent<Game.Objects.Object>(e))
        {
            return MissingKind.Object;
        }
        return em.HasComponent<Edge>(e) ? MissingKind.Net : MissingKind.Area;
    }

    /// <summary>ゲームの情報パネルの「削除」ボタンが使える条件に合わせた判定（成長型の建物だけ緩めている）。</summary>
    private static bool IsDeletable(EntityManager em, Entity e, MissingKind kind)
    {
        if (em.HasComponent<Game.Objects.OutsideConnection>(e))
        {
            return false;
        }
        switch (kind)
        {
            case MissingKind.Building:
                if (em.HasComponent<Game.Buildings.WaterPowered>(e))
                {
                    return false;
                }
                // 付属建物は本体ごとでないと消せない。ただし施設の拡張（ServiceUpgrade）は単独で消せる。
                return !em.HasComponent<Owner>(e) || em.HasComponent<Game.Buildings.ServiceUpgrade>(e);
            case MissingKind.Object:
                return em.HasComponent<Game.Objects.Static>(e) && !em.HasComponent<Native>(e) && !em.HasComponent<Owner>(e);
            default:
                return false;
        }
    }

    private static float3 PositionOf(EntityManager em, Entity e)
    {
        if (em.HasComponent<Transform>(e))
        {
            return em.GetComponentData<Transform>(e).m_Position;
        }
        if (em.HasComponent<Curve>(e))
        {
            // 区間の曲線の中ほど（ベジェ曲線の t=0.5）。
            var b = em.GetComponentData<Curve>(e).m_Bezier;
            return (b.a + 3f * b.b + 3f * b.c + b.d) / 8f;
        }
        if (em.HasComponent<Geometry>(e))
        {
            return em.GetComponentData<Geometry>(e).m_CenterPosition;
        }
        return float3.zero;
    }

    /// <summary>動作確認用（DebugMarkers の debug-missing / debug-delete）の自動操作。</summary>
    private void RunDebug()
    {
        var missing = DebugMarkers.Has("debug-missing");
        var delete = DebugMarkers.Has("debug-delete");
        if (!missing && !delete)
        {
            return;
        }
        var now = UnityEngine.Time.realtimeSinceStartup;
        if (now < m_DebugAt)
        {
            return;
        }
        switch (m_DebugStep)
        {
            case 0:
                Scan();
                Debug.Log($"{Mod.LogPrefix} missing before {Summary()} orphans={CountOrphans()}");
                var first = m_Result.Groups.FirstOrDefault(g => g.Kind != MissingKind.Area);
                if (first is not null)
                {
                    OnJump(first.Key, 0);
                }
                m_DebugAt = now + 8f;
                m_DebugStep = 1;
                break;
            case 1:
                UnityEngine.ScreenCapture.CaptureScreenshot(DebugMarkers.PathOf("debug-missing.png"));
                Debug.Log($"{Mod.LogPrefix} screenshot requested (missing)");
                m_DebugAt = now + 5f;
                m_DebugStep = delete ? 2 : 99;
                break;
            case 2:
                OnDelete(string.Join("\n", m_Result.Groups.Select(g => g.Key)));
                m_DebugAt = now + 10f;
                m_DebugStep = 3;
                break;
            case 3:
                Debug.Log($"{Mod.LogPrefix} missing after {Summary()} deleted={m_LastDeleted} orphans={CountOrphans()}");
                UnityEngine.ScreenCapture.CaptureScreenshot(DebugMarkers.PathOf("debug-deleted.png"));
                m_DebugStep = 99;
                break;
        }
    }

    private string Summary() => string.Join("; ", m_Result.Groups.Select(g => $"{g.PrefabName}[{MissingAssets.KindKey(g.Kind)}]={g.Instances.Count}/{g.DeletableCount}"));

    /// <summary>持ち主が存在しない Owner の数（削除の後始末が漏れていないかの確認用）。</summary>
    private int CountOrphans()
    {
        var em = EntityManager;
        using var query = em.CreateEntityQuery(new EntityQueryDesc
        {
            All = new[] { ComponentType.ReadOnly<Owner>() },
            None = new[] { ComponentType.ReadOnly<Temp>(), ComponentType.ReadOnly<Deleted>() },
        });
        using var owners = query.ToComponentDataArray<Owner>(Allocator.Temp);
        var n = 0;
        foreach (var o in owners)
        {
            if (o.m_Owner != Entity.Null && !em.Exists(o.m_Owner))
            {
                n++;
            }
        }
        return n;
    }
}
