// 計測専用の一時mod。経路探しの依頼を「依頼した処理（システム）」と「目的地の種類」ごとに数え、CSVへ書き出す。
//
// 目的: 大きな街で経路探しの待ち行列が飽和している原因（どの処理の依頼が多いか）を特定する。
// 設計:
// - ゲームの処理は一切変えない。PathfindSetupSystem の直前と直後に動く読み取り専用のシステムを追加するだけ。
// - PathfindSetupSystem は OnUpdate の先頭で前回の依頼一覧(m_ActionList)を片付け、そのフレームの新しい依頼を積む。
//   そのため直後に m_ActionList を読めば、そのフレームに受け付けた依頼だけを数えられる。
// - ただし受付待ちの列(m_ActiveQueues)が空のフレームは OnUpdate が先頭で戻り、一覧が前回のまま残る。
//   二重に数えないよう、直前のシステムで列の数を記録し、空だったフレームは数えない。
// 注意: 非公開フィールドを反射で読むため、ゲームの更新で名前が変わると動かない（その場合は何も数えずログに出す）。
using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using Game;
using Game.Modding;
using Game.Pathfind;
using Game.Simulation;
using Game.Common;
using Game.Citizens;
using Unity.Entities;
using UnityEngine;

namespace PathfindCount
{
    public sealed class Mod : IMod
    {
        public void OnLoad(UpdateSystem updateSystem)
        {
            updateSystem.UpdateBefore<PathfindCountBeforeSystem, PathfindSetupSystem>(SystemUpdatePhase.MainLoop);
            updateSystem.UpdateAfter<PathfindCountSystem, PathfindSetupSystem>(SystemUpdatePhase.MainLoop);
            // 修正は Mods\PathfindCount\fix.enabled があるときだけ有効にする（同じDLLで修正なし・ありを比べるため）。
            if (File.Exists(Path.Combine(Application.persistentDataPath, "Mods", "PathfindCount", "fix.enabled")))
            {
                updateSystem.UpdateAt<SwitchJobRateFixSystem>(SystemUpdatePhase.GameSimulation);
                Debug.Log("[PathfindCount] SwitchJobRate fix enabled");
            }
            updateSystem.UpdateBefore<JobSearchOutcomeSystem, FindJobSystem>(SystemUpdatePhase.GameSimulation);
            Debug.Log("[PathfindCount] loaded");
        }

        public void OnDispose() { }
    }

    /// <summary>
    /// 仕事探しの経路結果を、FindJobSystem が採用処理をする直前に分類して数える。
    /// - 見つからず: 経路の行き先が空（条件に合う職場に届かなかった）
    /// - 空きあり: 行き先の事業所に空きがある（このあと採用される見込み）
    /// - 満員: 行き先はあるが空きが無くなっていた
    /// 結果が出た求職エンティティは FindJobSystem の処理後に削除されるため、同じものを二重に数えないよう集合で除く。
    /// </summary>
    public partial class JobSearchOutcomeSystem : GameSystemBase
    {
        private EntityQuery m_Query;
        private readonly HashSet<Entity> m_Seen = new HashSet<Entity>();
        private readonly Dictionary<string, long> m_Counts = new Dictionary<string, long>();
        private float m_NextWrite;

        protected override void OnCreate()
        {
            base.OnCreate();
            m_Query = GetEntityQuery(
                ComponentType.ReadOnly<Game.Agents.JobSeeker>(),
                ComponentType.ReadOnly<PathInformation>(),
                ComponentType.ReadOnly<Owner>());
        }

        protected override void OnUpdate()
        {
            using (var entities = m_Query.ToEntityArray(Unity.Collections.Allocator.Temp))
            {
                foreach (var e in entities)
                {
                    var info = EntityManager.GetComponentData<PathInformation>(e);
                    if ((info.m_State & PathFlags.Pending) != 0 || !m_Seen.Add(e))
                    {
                        continue;
                    }
                    string result;
                    if (info.m_Destination == Entity.Null || !EntityManager.Exists(info.m_Destination))
                    {
                        result = "not-found";
                    }
                    else if (EntityManager.HasComponent<Game.Companies.FreeWorkplaces>(info.m_Destination)
                        && EntityManager.GetComponentData<Game.Companies.FreeWorkplaces>(info.m_Destination).Count > 0)
                    {
                        result = "vacancy";
                    }
                    else
                    {
                        result = "full";
                    }
                    var citizen = EntityManager.GetComponentData<Owner>(e).m_Owner;
                    var kind = "?";
                    var edu = -1;
                    if (EntityManager.Exists(citizen) && EntityManager.HasComponent<Citizen>(citizen))
                    {
                        var c = EntityManager.GetComponentData<Citizen>(citizen);
                        edu = c.GetEducationLevel();
                        kind = EntityManager.HasComponent<Worker>(citizen) ? "employed"
                            : (c.m_State & CitizenFlags.Commuter) != 0 ? "commuter" : "resident";
                    }
                    var key = $"{result},{kind},{edu}";
                    m_Counts.TryGetValue(key, out var n);
                    m_Counts[key] = n + 1;
                }
            }
            if (m_Seen.Count > 200000)
            {
                m_Seen.Clear();
            }
            if (UnityEngine.Time.realtimeSinceStartup >= m_NextWrite)
            {
                m_NextWrite = UnityEngine.Time.realtimeSinceStartup + 30f;
                try
                {
                    var lines = new List<string> { "count,result,kind,education" };
                    lines.AddRange(m_Counts.OrderByDescending(p => p.Value).Select(p => $"{p.Value},{p.Key}"));
                    File.WriteAllLines(Path.Combine(Application.persistentDataPath, "Mods", "PathfindCount", "job-outcome.csv"), lines);
                }
                catch (Exception ex)
                {
                    Debug.LogError("[PathfindCount] job outcome write failed: " + ex.Message);
                }
            }
        }
    }

    /// <summary>
    /// 転職探しの確率を説明どおりに戻す修正。
    /// CitizenFindJobSystem は「乱数(0〜1) &gt; m_SwitchJobRate」のときに働いている人の転職探しを行うため、
    /// 説明上 3.2%(0.032) のはずが約96.8%で実行される。値を 1 - 0.032 にして実行率を 3.2% にする。
    /// 設定値はプレハブ由来の単一エンティティにあり、ロードや設定の再適用で戻る可能性があるため毎回確認する。
    /// </summary>
    public partial class SwitchJobRateFixSystem : GameSystemBase
    {
        private const float Intended = 0.032f;
        private EntityQuery m_Query;
        private bool m_Logged;

        protected override void OnCreate()
        {
            base.OnCreate();
            m_Query = GetEntityQuery(ComponentType.ReadWrite<Game.Prefabs.CitizenParametersData>());
        }

        protected override void OnUpdate()
        {
            if (m_Query.IsEmptyIgnoreFilter)
            {
                return;
            }
            var data = m_Query.GetSingleton<Game.Prefabs.CitizenParametersData>();
            var target = 1f - Intended;
            if (data.m_SwitchJobRate != target)
            {
                if (!m_Logged)
                {
                    Debug.Log($"[PathfindCount] SwitchJobRate {data.m_SwitchJobRate} -> {target}");
                    m_Logged = true;
                }
                data.m_SwitchJobRate = target;
                m_Query.SetSingleton(data);
            }
        }
    }

    /// <summary>PathfindSetupSystem の直前に、受付待ちの列があるかを記録する。</summary>
    public partial class PathfindCountBeforeSystem : GameSystemBase
    {
        internal static bool HadQueues;
        private PathfindSetupSystem m_Setup;
        private FieldInfo m_ActiveQueues;

        protected override void OnCreate()
        {
            base.OnCreate();
            m_Setup = World.GetOrCreateSystemManaged<PathfindSetupSystem>();
            m_ActiveQueues = typeof(PathfindSetupSystem).GetField("m_ActiveQueues", BindingFlags.Instance | BindingFlags.NonPublic);
        }

        protected override void OnUpdate()
        {
            HadQueues = m_ActiveQueues != null && m_ActiveQueues.GetValue(m_Setup) is IList list && list.Count != 0;
        }
    }

    /// <summary>PathfindSetupSystem の直後に、そのフレームに受け付けた依頼を数える。</summary>
    public partial class PathfindCountSystem : GameSystemBase
    {
        private PathfindSetupSystem m_Setup;
        private SimulationSystem m_Simulation;
        private FieldInfo m_ActionList;
        private FieldInfo m_ItemSystem;
        private FieldInfo m_ItemAction;
        private FieldInfo m_ItemOwner;
        private readonly Dictionary<string, long> m_Counts = new Dictionary<string, long>();
        private uint m_FirstFrame;
        private long m_Total;
        private float m_NextWrite;
        private string m_Path;

        protected override void OnCreate()
        {
            base.OnCreate();
            m_Setup = World.GetOrCreateSystemManaged<PathfindSetupSystem>();
            m_Simulation = World.GetOrCreateSystemManaged<SimulationSystem>();
            const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public;
            m_ActionList = typeof(PathfindSetupSystem).GetField("m_ActionList", flags);
            var itemType = typeof(PathfindSetupSystem).GetNestedType("ActionListItem", flags);
            m_ItemSystem = itemType?.GetField("m_System", flags);
            m_ItemAction = itemType?.GetField("m_Action", flags);
            m_ItemOwner = itemType?.GetField("m_Owner", flags);
            // 出力先は Mods\PathfindCount。ゲームはmodのDLLをメモリから読むため Assembly.Location は空になる。
            m_Path = Path.Combine(Application.persistentDataPath, "Mods", "PathfindCount", "pathfind-count.csv");
            if (m_ActionList == null || m_ItemSystem == null || m_ItemAction == null)
            {
                Debug.LogError("[PathfindCount] fields not found; counting disabled");
            }
        }

        protected override void OnUpdate()
        {
            if (m_ActionList == null || m_ItemSystem == null || m_ItemAction == null)
            {
                return;
            }
            if (m_FirstFrame == 0)
            {
                m_FirstFrame = m_Simulation.frameIndex;
            }
            if (PathfindCountBeforeSystem.HadQueues && m_ActionList.GetValue(m_Setup) is IList list)
            {
                foreach (var item in list)
                {
                    var system = m_ItemSystem.GetValue(item)?.GetType().Name ?? "(none)";
                    var action = (PathfindAction)m_ItemAction.GetValue(item);
                    var data = action.readOnlyData;
                    var key = $"{system},{data.m_OriginType},{data.m_DestinationType},{data.m_Parameters.m_MaxResultCount}";
                    // 仕事探しは「働いている人の転職」と「失業者」を分ける。依頼の持ち主は求職エンティティで、その Owner が市民。
                    if (system == "FindJobSystem" && m_ItemOwner != null)
                    {
                        var seeker = (Entity)m_ItemOwner.GetValue(item);
                        var kind = "unknown";
                        if (EntityManager.Exists(seeker) && EntityManager.HasComponent<Owner>(seeker))
                        {
                            var citizen = EntityManager.GetComponentData<Owner>(seeker).m_Owner;
                            kind = EntityManager.Exists(citizen) && EntityManager.HasComponent<Worker>(citizen) ? "employed" : "unemployed";
                        }
                        key += "," + kind;
                    }
                    m_Counts.TryGetValue(key, out var n);
                    m_Counts[key] = n + 1;
                    m_Total++;
                }
            }
            // 実時間で30秒ごとに、累計を書き出す（ファイルは毎回上書き）
            if (UnityEngine.Time.realtimeSinceStartup >= m_NextWrite)
            {
                m_NextWrite = UnityEngine.Time.realtimeSinceStartup + 30f;
                Write();
            }
        }

        private void Write()
        {
            try
            {
                var lines = new List<string>
                {
                    $"# simFrames={m_Simulation.frameIndex - m_FirstFrame} total={m_Total} realtime={UnityEngine.Time.realtimeSinceStartup:F0}",
                    "count,system,origin,destination,maxResults",
                };
                lines.AddRange(m_Counts.OrderByDescending(p => p.Value).Select(p => $"{p.Value},{p.Key}"));
                File.WriteAllLines(m_Path, lines);
            }
            catch (Exception e)
            {
                Debug.LogError("[PathfindCount] write failed: " + e.Message);
            }
        }
    }
}
