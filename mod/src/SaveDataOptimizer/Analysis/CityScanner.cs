using Game.Agents;
using Game.Buildings;
using Game.Citizens;
using Game.Common;
using Game.Companies;
using Game.Tools;
using SaveDataOptimizer.Core;
using Unity.Collections;
using Unity.Entities;
using Transform = Game.Objects.Transform;

namespace SaveDataOptimizer.Analysis;

/// <summary>
/// 市民・世帯・職場を数えて <see cref="JobMarket"/> と世帯の集計を作る（読み取り専用）。
/// </summary>
/// <remarks>
/// 職探しの対象は、ゲームの CitizenFindJobSystem と同じ条件にそろえる:
/// 世帯に属し、働いておらず、学生・病気でなく、子どもと高齢者を除く人。
/// 観光客の世帯は除き、転出中の世帯の人も職探しをしないので除く。市外からの通勤者は地区に入れず別に数える。
/// 住まいの位置は、世帯の PropertyRenter（住んでいる建物）か、ホームレスなら仮の住まいの建物で取る。
/// メインスレッドで全件をたどるため重い。PanelUISystem がパネルを開いている間だけ、間隔を空けて呼ぶ。
/// </remarks>
internal sealed class CityScanner
{
    private readonly EntityManager m_EntityManager;
    private readonly EntityQuery m_JobSeekers;
    private readonly EntityQuery m_Workplaces;
    private readonly EntityQuery m_Citizens;
    private readonly EntityQuery m_Homeless;
    private readonly EntityQuery m_MovingAway;

    public CityScanner(EntityManager entityManager, EntityQuery jobSeekers, EntityQuery workplaces, EntityQuery citizens, EntityQuery homeless, EntityQuery movingAway)
    {
        m_EntityManager = entityManager;
        m_JobSeekers = jobSeekers;
        m_Workplaces = workplaces;
        m_Citizens = citizens;
        m_Homeless = homeless;
        m_MovingAway = movingAway;
    }

    /// <summary>
    /// CitizenFindJobSystem の失業者の対象と同じ条件のクエリ。PanelUISystem.OnCreate から作る。
    /// </summary>
    public static EntityQueryDesc JobSeekersDesc() => new()
    {
        All = new[] { ComponentType.ReadOnly<Citizen>(), ComponentType.ReadOnly<HouseholdMember>() },
        None = new[]
        {
            ComponentType.ReadOnly<Temp>(),
            ComponentType.ReadOnly<Worker>(),
            ComponentType.ReadOnly<Game.Citizens.Student>(),
            ComponentType.ReadOnly<HealthProblem>(),
            ComponentType.ReadOnly<Deleted>(),
        },
    };

    public static EntityQueryDesc WorkplacesDesc() => new()
    {
        All = new[] { ComponentType.ReadOnly<FreeWorkplaces>() },
        None = new[] { ComponentType.ReadOnly<Temp>(), ComponentType.ReadOnly<Deleted>() },
    };

    public static EntityQueryDesc CitizensDesc() => new()
    {
        All = new[] { ComponentType.ReadOnly<Citizen>() },
        None = new[] { ComponentType.ReadOnly<Temp>(), ComponentType.ReadOnly<Deleted>() },
    };

    public static EntityQueryDesc HomelessDesc() => new()
    {
        All = new[] { ComponentType.ReadOnly<Household>(), ComponentType.ReadOnly<HomelessHousehold>() },
        None = new[] { ComponentType.ReadOnly<Temp>(), ComponentType.ReadOnly<Deleted>() },
    };

    public static EntityQueryDesc MovingAwayDesc() => new()
    {
        All = new[] { ComponentType.ReadOnly<Household>(), ComponentType.ReadOnly<MovingAway>() },
        None = new[] { ComponentType.ReadOnly<Temp>(), ComponentType.ReadOnly<Deleted>() },
    };

    public void Fill(LoadReport report)
    {
        report.Citizens = m_Citizens.CalculateEntityCount();
        report.HomelessHouseholds = m_Homeless.CalculateEntityCount();
        report.MovingAwayHouseholds = m_MovingAway.CalculateEntityCount();
        report.Jobs = ScanJobs();
    }

    private JobMarket ScanJobs()
    {
        var market = new JobMarket();
        var em = m_EntityManager;
        using (var citizens = m_JobSeekers.ToComponentDataArray<Citizen>(Allocator.Temp))
        using (var members = m_JobSeekers.ToComponentDataArray<HouseholdMember>(Allocator.Temp))
        {
            for (var i = 0; i < citizens.Length; i++)
            {
                var citizen = citizens[i];
                var age = citizen.GetAge();
                if (age == CitizenAge.Child || age == CitizenAge.Elderly)
                {
                    continue;
                }
                var household = members[i].m_Household;
                if (!em.Exists(household) || em.HasComponent<TouristHousehold>(household) || em.HasComponent<MovingAway>(household))
                {
                    continue;
                }
                var level = citizen.GetEducationLevel();
                if ((citizen.m_State & CitizenFlags.Commuter) != CitizenFlags.None)
                {
                    market.UnemployedCommuters++;
                    continue;
                }
                if (TryHomePosition(em, household, out var p))
                {
                    market.AddUnemployed(level, p.x, p.y, p.z);
                }
                else
                {
                    market.AddUnemployedWithoutPosition(level);
                }
            }
        }

        using (var entities = m_Workplaces.ToEntityArray(Allocator.Temp))
        using (var free = m_Workplaces.ToComponentDataArray<FreeWorkplaces>(Allocator.Temp))
        {
            for (var i = 0; i < entities.Length; i++)
            {
                var f = free[i];
                if (f.Count == 0)
                {
                    continue;
                }
                var hasPosition = TryWorkplacePosition(em, entities[i], out var p);
                for (var level = 0; level < JobMarket.Levels; level++)
                {
                    int count = f.GetFree(level);
                    if (hasPosition)
                    {
                        market.AddVacancies(level, count, p.x, p.y, p.z);
                    }
                    else
                    {
                        market.AddVacanciesWithoutPosition(level, count);
                    }
                }
            }
        }
        return market;
    }

    private static bool TryHomePosition(EntityManager em, Entity household, out Unity.Mathematics.float3 position)
    {
        var building = Entity.Null;
        if (em.HasComponent<PropertyRenter>(household))
        {
            building = em.GetComponentData<PropertyRenter>(household).m_Property;
        }
        else if (em.HasComponent<HomelessHousehold>(household))
        {
            building = em.GetComponentData<HomelessHousehold>(household).m_TempHome;
        }
        return TryPosition(em, building, out position);
    }

    /// <summary>職場（会社や公共施設）の位置。建物そのものなら Transform、会社なら借りている建物の Transform。</summary>
    private static bool TryWorkplacePosition(EntityManager em, Entity workplace, out Unity.Mathematics.float3 position)
    {
        if (TryPosition(em, workplace, out position))
        {
            return true;
        }
        if (em.HasComponent<PropertyRenter>(workplace))
        {
            return TryPosition(em, em.GetComponentData<PropertyRenter>(workplace).m_Property, out position);
        }
        return false;
    }

    private static bool TryPosition(EntityManager em, Entity entity, out Unity.Mathematics.float3 position)
    {
        if (entity != Entity.Null && em.Exists(entity) && em.HasComponent<Transform>(entity))
        {
            position = em.GetComponentData<Transform>(entity).m_Position;
            return true;
        }
        position = default;
        return false;
    }
}
