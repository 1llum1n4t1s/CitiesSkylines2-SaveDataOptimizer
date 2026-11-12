using System.Collections;
using System.Reflection;
using Game;
using Game.Citizens;
using Game.Common;
using Game.Pathfind;
using Game.Simulation;
using CityCheckup.Core;
using Unity.Entities;
using UnityEngine;

namespace CityCheckup.Analysis;

/// <summary>
/// 経路探しの依頼を、依頼したシステムごとに数える（ゲームの処理は変えない、読み取り専用）。
/// </summary>
/// <remarks>
/// PathfindSetupSystem は OnUpdate の先頭で前回の依頼一覧（m_ActionList）を片付け、そのフレームの依頼を積む。
/// そのため直後に m_ActionList を読めば、そのフレームに受け付けた依頼だけを数えられる。
/// ただし受付待ちの列（m_ActiveQueues）が空のフレームは先頭で戻って一覧が前回のまま残るので、
/// 直前に動く <see cref="PathfindDemandBeforeSystem"/> で列の有無を記録し、空のフレームは数えない。
/// 非公開フィールドを反射で読むため、ゲームの更新で名前が変わると数えられなくなる（その場合はログに 1 度出して止まる）。
/// 反射での読み取りは軽くないので、パネルを開いている間（<see cref="Collecting"/>）だけ動かす。
/// </remarks>
public sealed partial class PathfindDemandSystem : GameSystemBase
{
    private const BindingFlags Flags = BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public;

    private PathfindSetupSystem m_Setup = null!;
    private FieldInfo? m_ActionList;
    private FieldInfo? m_ItemSystem;
    private FieldInfo? m_ItemOwner;

    /// <summary>パネルが開いている間だけ true にする（PanelUISystem が切り替える）。</summary>
    public static bool Collecting { get; set; }

    /// <summary>集計結果。PanelUISystem が区間を締めて表示に使う。</summary>
    public static PathfindTally Tally { get; } = new();

    /// <summary>ゲームの内部が想定と違い、数えられない状態か。</summary>
    public static bool Unsupported { get; private set; }

    protected override void OnCreate()
    {
        base.OnCreate();
        m_Setup = World.GetOrCreateSystemManaged<PathfindSetupSystem>();
        m_ActionList = typeof(PathfindSetupSystem).GetField("m_ActionList", Flags);
        var itemType = typeof(PathfindSetupSystem).GetNestedType("ActionListItem", Flags);
        m_ItemSystem = itemType?.GetField("m_System", Flags);
        m_ItemOwner = itemType?.GetField("m_Owner", Flags);
        if (m_ActionList is null || m_ItemSystem is null)
        {
            Unsupported = true;
            Debug.LogWarning($"{Mod.LogPrefix} PathfindSetupSystem の内部が想定と違うため、経路探しの依頼は数えません");
        }
    }

    protected override void OnUpdate()
    {
        if (!Collecting || Unsupported || !PathfindDemandBeforeSystem.HadQueues)
        {
            return;
        }
        if (m_ActionList!.GetValue(m_Setup) is not IList list)
        {
            return;
        }
        foreach (var item in list)
        {
            var system = m_ItemSystem!.GetValue(item)?.GetType().Name ?? string.Empty;
            bool? employed = null;
            // 職探しは「働いている人の転職」と「失業者」を分ける。依頼の持ち主は求職エンティティで、その Owner が市民。
            if (system == "FindJobSystem" && m_ItemOwner is not null)
            {
                var seeker = (Entity)m_ItemOwner.GetValue(item);
                if (EntityManager.Exists(seeker) && EntityManager.HasComponent<Owner>(seeker))
                {
                    var citizen = EntityManager.GetComponentData<Owner>(seeker).m_Owner;
                    employed = EntityManager.Exists(citizen) && EntityManager.HasComponent<Worker>(citizen);
                }
            }
            Tally.Add(PathfindSources.Classify(system, employed));
        }
    }
}

/// <summary>PathfindSetupSystem の直前に、受付待ちの列があるかだけを記録する。</summary>
public sealed partial class PathfindDemandBeforeSystem : GameSystemBase
{
    private PathfindSetupSystem m_Setup = null!;
    private FieldInfo? m_ActiveQueues;

    /// <summary>このフレームの PathfindSetupSystem が依頼一覧を作り直すか。</summary>
    public static bool HadQueues { get; private set; }

    protected override void OnCreate()
    {
        base.OnCreate();
        m_Setup = World.GetOrCreateSystemManaged<PathfindSetupSystem>();
        m_ActiveQueues = typeof(PathfindSetupSystem).GetField("m_ActiveQueues", BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public);
    }

    protected override void OnUpdate()
    {
        HadQueues = PathfindDemandSystem.Collecting
            && m_ActiveQueues?.GetValue(m_Setup) is ICollection { Count: > 0 };
    }
}
