using System;
using System.Collections.Generic;
using System.Linq;
using Colossal.Serialization.Entities;
using Colossal.UI.Binding;
using Game;
using Game.Common;
using Game.Net;
using Game.Objects;
using Game.Rendering;
using Game.Simulation;
using Game.Tools;
using Game.UI;
using Game.UI.InGame;
using Game.Vehicles;
using SaveDataOptimizer.Core;
using SaveDataOptimizer.Ui;
using Unity.Collections;
using Unity.Entities;
using Debug = UnityEngine.Debug;
using Transform = Game.Objects.Transform;

namespace SaveDataOptimizer.Vehicles;

/// <summary>
/// 線路や駐車場を撤去したあとに残り続ける列車・車を探し、選ばれたものを消す。
/// </summary>
/// <remarks>
/// 判定（<see cref="StrandReason"/>）:
/// - 車線が無い: 車両が今いる車線（走行中は CarCurrentLane / TrainCurrentLane の先頭、駐車中は ParkedCar / ParkedTrain）
///   の Entity が存在しない・削除済み・車線でない。撤去した線路や駐車場に乗ったまま残った車両がこれに当たる。
/// - 車線に載っていない: 車線は残っているが、その車線の LaneObject（車線上の物の一覧）に車両が入っていない。
///   ゲームは車線上の物をこの一覧で扱うため、入っていない車両は交通から外れて置き去りになっている。
/// - 動いていない: 走行中のはずが一定時間ほとんど動いていない。渋滞や停車も含むので参考扱い（既定では選ばない）。
/// 列車と連結車は編成の先頭（Controller が自分）でまとめて 1 件にし、消すときは編成の全車両を消す。
/// 画面に出ていない車両（Unspawned。建物内の駐車など）はゲームの通常の状態なので対象にしない。
///
/// 消し方はゲームが車両を片付けるときと同じく、EndFrameBarrier で Deleted を付ける（持ち主の世帯などの参照はゲームが整理する）。
/// 探索は全車両をたどるので重い。UI の「探す」ボタン（または削除の後）でだけ動かす。
/// </remarks>
public sealed partial class StrandedVehicleSystem : UISystemBase
{
    private const string Group = PanelUISystem.Group;
    private const int RescanDelayFrames = 10;

    private SimulationSystem m_Simulation = null!;
    private CameraUpdateSystem m_Camera = null!;
    private SelectedInfoUISystem m_SelectedInfo = null!;
    private EndFrameBarrier m_EndFrameBarrier = null!;
    private EntityQuery m_Vehicles;
    private ValueBinding<string> m_Binding = null!;

    private StrandedVehicles m_Result = new();
    private readonly StuckTracker m_Stuck = new();
    private readonly FlagTracker m_Flags = new();
    private string m_State = "idle";
    private int? m_LastDeleted;
    private bool m_InGame;
    private int m_RescanCountdown;

    // 動作確認用（DebugMarkers の debug-vehicles / debug-vehicles-delete）の進行。
    private int m_DebugStep;
    private float m_DebugAt;

    public override GameMode gameMode => GameMode.Game;

    protected override void OnCreate()
    {
        base.OnCreate();
        m_Simulation = World.GetOrCreateSystemManaged<SimulationSystem>();
        m_Camera = World.GetOrCreateSystemManaged<CameraUpdateSystem>();
        m_SelectedInfo = World.GetOrCreateSystemManaged<SelectedInfoUISystem>();
        m_EndFrameBarrier = World.GetOrCreateSystemManaged<EndFrameBarrier>();
        m_Vehicles = GetEntityQuery(new EntityQueryDesc
        {
            All = new[] { ComponentType.ReadOnly<Vehicle>(), ComponentType.ReadOnly<Transform>() },
            None = new[] { ComponentType.ReadOnly<Temp>(), ComponentType.ReadOnly<Deleted>() },
        });
        m_Binding = new ValueBinding<string>(Group, "vehicles", m_Result.ToJson(m_State, null));
        AddBinding(m_Binding);
        AddBinding(new TriggerBinding(Group, "scanVehicles", Scan));
        AddBinding(new TriggerBinding<string, int>(Group, "jumpVehicle", OnJump));
        AddBinding(new TriggerBinding<string>(Group, "deleteVehicles", OnDelete));
    }

    protected override void OnGamePreload(Purpose purpose, GameMode mode)
    {
        base.OnGamePreload(purpose, mode);
        m_InGame = false;
        m_Result = new StrandedVehicles();
        m_Stuck.Reset();
        m_Flags.Reset();
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
        if (m_RescanCountdown > 0 && --m_RescanCountdown == 0)
        {
            Scan();
        }
        RunDebug();
    }

    private void Scan() => Scan(null);

    /// <param name="census">動作確認用。車両の状態の内訳を数える先（null なら数えない）。</param>
    private void Scan(Dictionary<string, int>? census)
    {
        var em = EntityManager;
        var result = new StrandedVehicles();
        var frame = m_Simulation.frameIndex;
        var next = new Dictionary<long, (float X, float Y, float Z, uint SinceFrame)>();
        var flagged = new Dictionary<long, uint>();
        using var vehicles = m_Vehicles.ToEntityArray(Allocator.Temp);
        foreach (var e in vehicles)
        {
            // 編成（列車・連結車）は先頭車でまとめる。
            if (em.HasComponent<Controller>(e) && em.GetComponentData<Controller>(e).m_Controller != e)
            {
                continue;
            }
            var kind = em.HasComponent<Train>(e) ? VehicleKind.Train : em.HasComponent<Car>(e) ? VehicleKind.Car : VehicleKind.Other;
            var hidden = em.HasComponent<Unspawned>(e);
            var parked = em.HasComponent<ParkedCar>(e) || em.HasComponent<ParkedTrain>(e);
            if (!TryCurrentLane(em, e, out var lane))
            {
                Count(census, kind, parked, hidden, "noLaneComponent");
                continue;
            }
            result.Scanned++;
            var state = LaneState(em, lane, e);
            if (hidden)
            {
                Count(census, kind, parked, hidden, state);
                continue;
            }
            var relation = Relation(em, e, out var relationReason);
            Count(census, kind, parked, hidden, state + "/" + relation);
            var p = em.GetComponentData<Transform>(e).m_Position;
            var v = new StrandedVehicle(e.Index, e.Version, p.x, p.y, p.z, parked, hidden);
            switch (state)
            {
                // 「車線でない物」（ヘリポートや建物の出入口など）にいるのは正常なので対象にしない（ゆろしてぃで確認）。
                case "missing":
                case "deleted":
                    result.Add(StrandReason.LaneGone, kind, v);
                    continue;
                case "notListed":
                    result.Add(StrandReason.NotOnLane, kind, v);
                    continue;
            }
            // 列車の「路線に属さない」は走行中だけを見る（車庫に停めてある列車は路線を持たないのが普通）。
            if (relationReason == StrandReason.OwnerGone || (relationReason == StrandReason.NoRoute && !parked))
            {
                // 一時的な状態を除くため、一定時間続いたものだけを取り残しにする。
                if (m_Flags.Observe(StuckTracker.Id(e.Index, e.Version), frame, flagged) >= StuckTracker.StuckFrames)
                {
                    result.Add(relationReason.Value, kind, v);
                }
                else
                {
                    result.Pending++;
                }
                continue;
            }
            // 前の車両や信号などに塞がれて止まっているのは渋滞なので除く（ゆろしてぃの路面電車・車の渋滞で確認）。
            if (!parked && em.HasComponent<Moving>(e) && !IsBlocked(em, e))
            {
                var still = m_Stuck.Observe(StuckTracker.Id(e.Index, e.Version), p.x, p.y, p.z, frame, next);
                if (still >= StuckTracker.StuckFrames)
                {
                    result.Add(StrandReason.Stuck, kind, v);
                }
            }
        }
        m_Stuck.Commit(next);
        m_Flags.Commit(flagged);
        m_Result = result;
        m_State = "ready";
        m_Binding.Update(m_Result.ToJson(m_State, m_LastDeleted));
        Debug.Log($"{Mod.LogPrefix} vehicles scan: {result.Scanned} on lanes, {result.Total} stranded ({Summary()})");
    }

    private static bool IsBlocked(EntityManager em, Entity e)
    {
        if (!em.HasComponent<Blocker>(e))
        {
            return false;
        }
        var b = em.GetComponentData<Blocker>(e).m_Blocker;
        return b != Entity.Null && em.Exists(b);
    }

    /// <summary>車両が今いる（駐車している）車線。走行中は先頭の位置の車線。</summary>
    private static bool TryCurrentLane(EntityManager em, Entity e, out Entity lane)
    {
        if (em.HasComponent<ParkedCar>(e))
        {
            lane = em.GetComponentData<ParkedCar>(e).m_Lane;
            return true;
        }
        if (em.HasComponent<ParkedTrain>(e))
        {
            lane = em.GetComponentData<ParkedTrain>(e).m_FrontLane;
            return true;
        }
        if (em.HasComponent<CarCurrentLane>(e))
        {
            lane = em.GetComponentData<CarCurrentLane>(e).m_Lane;
            return true;
        }
        if (em.HasComponent<TrainCurrentLane>(e))
        {
            lane = em.GetComponentData<TrainCurrentLane>(e).m_Front.m_Lane;
            return true;
        }
        if (em.HasComponent<WatercraftCurrentLane>(e))
        {
            lane = em.GetComponentData<WatercraftCurrentLane>(e).m_Lane;
            return true;
        }
        if (em.HasComponent<AircraftCurrentLane>(e))
        {
            lane = em.GetComponentData<AircraftCurrentLane>(e).m_Lane;
            return true;
        }
        lane = Entity.Null;
        return false;
    }

    /// <summary>
    /// 車線の状態: null（車線なし）/ missing（存在しない）/ deleted（削除済み）/ notLane（車線でない物）/
    /// notListed（車線上の物の一覧に入っていない）/ noList（一覧を持たない車線）/ ok。
    /// </summary>
    private static string LaneState(EntityManager em, Entity lane, Entity vehicle)
    {
        if (lane == Entity.Null)
        {
            return "null";
        }
        if (!em.Exists(lane))
        {
            return "missing";
        }
        if (em.HasComponent<Deleted>(lane))
        {
            return "deleted";
        }
        if (!em.HasComponent<Lane>(lane))
        {
            return "notLane";
        }
        if (!em.HasBuffer<LaneObject>(lane))
        {
            return "noList";
        }
        var objects = em.GetBuffer<LaneObject>(lane, true);
        // 列車は編成のどの車両で登録されているか分からないので、編成のいずれかが載っていればよい。
        DynamicBuffer<LayoutElement> layout = default;
        var hasLayout = em.HasBuffer<LayoutElement>(vehicle);
        if (hasLayout)
        {
            layout = em.GetBuffer<LayoutElement>(vehicle, true);
        }
        for (var i = 0; i < objects.Length; i++)
        {
            var o = objects[i].m_LaneObject;
            if (o == vehicle)
            {
                return "ok";
            }
            if (hasLayout)
            {
                for (var j = 0; j < layout.Length; j++)
                {
                    if (layout[j].m_Vehicle == o)
                    {
                        return "ok";
                    }
                }
            }
        }
        return "notListed";
    }

    /// <summary>
    /// 車両と持ち主・運転者・路線とのつながりを調べ、表示用の短い説明と、取り残しと判断した理由（無ければ null）を返す。
    /// </summary>
    /// <remarks>
    /// ゆーくんが見つけた症状に合わせた判定:
    /// - 車: 持ち主（世帯）が消えた・誰もいない、または運転していた人（PersonalCar の Keeper）が亡くなって消えた。
    /// - 列車: 公共交通の列車なのに路線（CurrentRoute）が無く、車庫へ帰る途中（Returning）でもない。
    /// 業務車両の持ち主（車庫の建物）が消えた場合も OwnerGone にする。
    /// </remarks>
    private static string Relation(EntityManager em, Entity e, out StrandReason? reason)
    {
        reason = null;
        var owner = "none";
        if (em.HasComponent<Owner>(e))
        {
            var o = em.GetComponentData<Owner>(e).m_Owner;
            if (o == Entity.Null)
            {
                owner = "null";
            }
            else if (!em.Exists(o) || em.HasComponent<Deleted>(o))
            {
                owner = "gone";
            }
            else if (em.HasComponent<Game.Citizens.Household>(o))
            {
                owner = em.HasBuffer<Game.Citizens.HouseholdCitizen>(o) && em.GetBuffer<Game.Citizens.HouseholdCitizen>(o, true).Length == 0 ? "empty" : "household";
            }
            else
            {
                owner = "other";
            }
        }
        if (owner == "gone" || owner == "empty")
        {
            reason = StrandReason.OwnerGone;
        }

        if (em.HasComponent<PersonalCar>(e))
        {
            var pc = em.GetComponentData<PersonalCar>(e);
            var keeper = pc.m_Keeper == Entity.Null ? "null" : em.Exists(pc.m_Keeper) && !em.HasComponent<Deleted>(pc.m_Keeper) ? "ok" : "gone";
            if (keeper == "gone")
            {
                reason = StrandReason.OwnerGone;
            }
            return $"pcar:o={owner},k={keeper}";
        }
        if (em.HasComponent<PublicTransport>(e))
        {
            var pt = em.GetComponentData<PublicTransport>(e);
            var route = "none";
            if (em.HasComponent<Game.Routes.CurrentRoute>(e))
            {
                var r = em.GetComponentData<Game.Routes.CurrentRoute>(e).m_Route;
                route = r != Entity.Null && em.Exists(r) && !em.HasComponent<Deleted>(r) ? "ok" : "gone";
            }
            var returning = (pt.m_State & PublicTransportFlags.Returning) != 0;
            if (reason is null && route != "ok" && !returning && em.HasComponent<Train>(e))
            {
                reason = StrandReason.NoRoute;
            }
            return $"pt:o={owner},r={route}{(returning ? ",ret" : "")}";
        }
        if (em.HasComponent<CargoTransport>(e))
        {
            return $"cargo:o={owner}";
        }
        return $"o={owner}";
    }

    private static void Count(Dictionary<string, int>? census, VehicleKind kind, bool parked, bool hidden, string state)
    {
        if (census is null)
        {
            return;
        }
        var key = $"{StrandedVehicles.KindKey(kind)}/{(parked ? "parked" : "moving")}/{(hidden ? "hidden" : "shown")}/{state}";
        census.TryGetValue(key, out var n);
        census[key] = n + 1;
    }

    private void OnJump(string key, int index)
    {
        var list = m_Result.In(key);
        if (list.Count == 0)
        {
            return;
        }
        var v = list[((index % list.Count) + list.Count) % list.Count];
        CameraJump.To(m_Camera, v.X, v.Y, v.Z);
        var entity = new Entity { Index = v.EntityIndex, Version = v.EntityVersion };
        if (EntityManager.Exists(entity))
        {
            m_SelectedInfo.SetSelection(entity);
        }
    }

    /// <param name="keys">消すグループの Key を改行でつないだもの。</param>
    private void OnDelete(string keys)
    {
        var em = EntityManager;
        var buffer = m_EndFrameBarrier.CreateCommandBuffer();
        var count = 0;
        foreach (var v in m_Result.InAll(keys.Split(new[] { '\n' }, StringSplitOptions.RemoveEmptyEntries)))
        {
            var e = new Entity { Index = v.EntityIndex, Version = v.EntityVersion };
            // 一覧を作った後に消えたり別の物に変わったりしていないか、消す直前に確かめ直す。
            if (!em.Exists(e) || em.HasComponent<Deleted>(e) || !em.HasComponent<Vehicle>(e))
            {
                continue;
            }
            if (em.HasBuffer<LayoutElement>(e))
            {
                var layout = em.GetBuffer<LayoutElement>(e, true);
                for (var i = 0; i < layout.Length; i++)
                {
                    var car = layout[i].m_Vehicle;
                    if (car != e && em.Exists(car) && !em.HasComponent<Deleted>(car))
                    {
                        buffer.AddComponent<Deleted>(car);
                    }
                }
            }
            buffer.AddComponent<Deleted>(e);
            count++;
        }
        m_LastDeleted = count;
        m_State = "deleting";
        m_Binding.Update(m_Result.ToJson(m_State, m_LastDeleted));
        m_RescanCountdown = RescanDelayFrames;
        Debug.Log($"{Mod.LogPrefix} vehicles delete: {count}");
    }

    /// <summary>動作確認用: 車両の状態（速度・車線の持ち主・つながり）を 1 行で説明する。</summary>
    private string Describe(StrandedVehicle v)
    {
        var em = EntityManager;
        var e = new Entity { Index = v.EntityIndex, Version = v.EntityVersion };
        if (!em.Exists(e))
        {
            return "gone";
        }
        var speed = em.HasComponent<Moving>(e) ? Unity.Mathematics.math.length(em.GetComponentData<Moving>(e).m_Velocity) : -1f;
        TryCurrentLane(em, e, out var lane);
        var laneOwner = lane != Entity.Null && em.Exists(lane) && em.HasComponent<Owner>(lane) ? em.GetComponentData<Owner>(lane).m_Owner : Entity.Null;
        var prefab = em.HasComponent<Game.Prefabs.PrefabRef>(e) ? em.GetComponentData<Game.Prefabs.PrefabRef>(e).m_Prefab : Entity.Null;
        var name = prefab != Entity.Null ? World.GetOrCreateSystemManaged<Game.Prefabs.PrefabSystem>().GetPrefabName(prefab) : "?";
        return $"prefab={name} speed={speed:0.00} lane={lane.Index} laneOwner={laneOwner.Index} rel={Relation(em, e, out _)} blocked={em.HasComponent<Blocker>(e)}";
    }

    private string Summary() => string.Join("; ", m_Result.Groups.Select(g => $"{StrandedVehicles.Key(g.Reason, g.Kind)}={g.Vehicles.Count}(parked {g.Vehicles.Count(v => v.Parked)})"));

    /// <summary>
    /// 動作確認用の自動操作。debug-vehicles: 探索して状態の内訳をログに書き、約 2.5 分後にもう一度探して「動いていない」も数え、
    /// 見つかった最初の車両へカメラを移して画面を保存する。debug-vehicles-delete があれば、そのあと「車線が無い」「車線に載っていない」を消す。
    /// </summary>
    private void RunDebug()
    {
        if (!DebugMarkers.Has("debug-vehicles"))
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
            case 1:
                var census = new Dictionary<string, int>();
                Scan(census);
                Debug.Log($"{Mod.LogPrefix} vehicles census#{m_DebugStep} " + string.Join(", ", census.OrderBy(p => p.Key).Select(p => $"{p.Key}={p.Value}")));
                foreach (var g in m_Result.Groups)
                {
                    var sample = string.Join(" ", g.Vehicles.Take(5).Select(v => $"({v.X:0},{v.Y:0},{v.Z:0}{(v.Parked ? ",P" : "")})"));
                    Debug.Log($"{Mod.LogPrefix} vehicles sample {StrandedVehicles.Key(g.Reason, g.Kind)}: {sample}");
                }
                // 2 回目の探索は、一定時間続いたかの判定（StuckFrames）がそろうまで待つ。
                m_DebugAt = now + 150f;
                m_DebugStep++;
                if (m_DebugStep == 2)
                {
                    // パネル（一覧）の画面を先に撮る。見に行くと選択中になり、パネルが畳まれるため。
                    UnityEngine.ScreenCapture.CaptureScreenshot(DebugMarkers.PathOf("debug-vehicles.png"));
                    m_DebugAt = now + 3f;
                }
                break;
            case 2:
                // 確実な取り残しを優先し、無ければ「動いていない」列車、次に車を見に行く。
                var first = m_Result.Groups.FirstOrDefault(g => g.Reason != StrandReason.Stuck);
                if (first.Vehicles is null)
                {
                    first = m_Result.Groups.OrderBy(g => g.Kind == VehicleKind.Train ? 0 : 1).FirstOrDefault();
                }
                if (first.Vehicles is not null)
                {
                    // 塞がれていない（渋滞でない）車両を優先して見に行く。
                    var key = StrandedVehicles.Key(first.Reason, first.Kind);
                    var index = 0;
                    for (var i = 0; i < first.Vehicles.Count; i++)
                    {
                        var d = Describe(first.Vehicles[i]);
                        Debug.Log($"{Mod.LogPrefix} vehicles detail {key}#{i} ({first.Vehicles[i].X:0},{first.Vehicles[i].Z:0}) {d}");
                        if (index == 0 && !d.Contains("blocked=True"))
                        {
                            index = i;
                        }
                    }
                    OnJump(key, index);
                    Debug.Log($"{Mod.LogPrefix} vehicles jump {key}#{index}");
                }
                m_DebugAt = now + 6f;
                m_DebugStep = 5;
                break;
            case 5:
                UnityEngine.ScreenCapture.CaptureScreenshot(DebugMarkers.PathOf("debug-vehicles-jump.png"));
                Debug.Log($"{Mod.LogPrefix} screenshots requested (vehicles)");
                m_DebugAt = now + 3f;
                m_DebugStep = DebugMarkers.Has("debug-vehicles-delete") ? 3 : 99;
                break;
            case 3:
                OnDelete(string.Join("\n", m_Result.Groups.Where(g => g.Reason != StrandReason.Stuck).Select(g => StrandedVehicles.Key(g.Reason, g.Kind))));
                m_DebugAt = now + 10f;
                m_DebugStep = 4;
                break;
            case 4:
                Debug.Log($"{Mod.LogPrefix} vehicles after delete: {Summary()} deleted={m_LastDeleted}");
                m_DebugStep = 99;
                break;
        }
    }
}
