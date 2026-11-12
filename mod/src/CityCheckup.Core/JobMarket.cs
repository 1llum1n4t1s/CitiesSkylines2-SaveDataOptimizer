using System;
using System.Collections.Generic;
using System.Linq;

namespace CityCheckup.Core;

/// <summary>
/// 失業者と空き職場を学歴の段階・地区ごとに数え、「職が足りない」「職が遠い」を判定する。
/// </summary>
/// <remarks>
/// ゆろしてぃの調査では、失業者が何度も職探しをすること（経路探しの約 1/3）が負荷の主因で、
/// その背景は (1) 空き職場の不足 と (2) 失業者の多い地区の近くに空き職場が無いこと だった。
/// ここではその 2 点を、ゲームを動かしながら確かめられる形で数える。
/// 学歴の段階は 0（教育なし）〜4（最高）で、ゲームの求職と同じく「自分の学歴以下の空き」に就ける前提で扱う。
/// </remarks>
public sealed class JobMarket
{
    public const int Levels = 5;

    /// <summary>地区の一辺（メートル）。調査時の 2 km 区画に合わせる。</summary>
    public const float CellSize = 2000f;

    private readonly Dictionary<(int X, int Z), Cell> m_Cells = new();

    /// <summary>学歴の段階ごとの、職探しの対象になる失業者の数（通勤者・観光客・転出中は含めない）。</summary>
    public long[] Unemployed { get; } = new long[Levels];

    /// <summary>学歴の段階ごとの空き職場の数。</summary>
    public long[] Vacancies { get; } = new long[Levels];

    /// <summary>市外から通勤してくる人のうち職の無い人。市内の地区には数えない。</summary>
    public long UnemployedCommuters { get; set; }

    public void AddUnemployed(int level, float x, float y, float z)
    {
        level = Clamp(level);
        Unemployed[level]++;
        var cell = CellAt(x, z);
        cell.Unemployed++;
        cell.AddPoint(x, y, z);
    }

    /// <summary>住まいの位置が分からない失業者（全体の数にだけ入れる）。</summary>
    public void AddUnemployedWithoutPosition(int level) => Unemployed[Clamp(level)]++;

    public void AddVacancies(int level, int count, float x, float y, float z)
    {
        if (count <= 0)
        {
            return;
        }
        level = Clamp(level);
        Vacancies[level] += count;
        var cell = CellAt(x, z);
        cell.Vacancies += count;
        cell.AddPoint(x, y, z);
    }

    public void AddVacanciesWithoutPosition(int level, int count)
    {
        if (count > 0)
        {
            Vacancies[Clamp(level)] += count;
        }
    }

    public long TotalUnemployed => Unemployed.Sum();

    public long TotalVacancies => Vacancies.Sum();

    /// <summary>
    /// 学歴の段階ごとに、その段階の人が就ける空き（自分の段階以下の空きの合計）。
    /// ゲームの求職（CitizenFindJobSystem）は、学歴以下の空きが 1 つも無いと職探しをしない。
    /// </summary>
    public long[] ReachableVacancies()
    {
        var result = new long[Levels];
        long sum = 0;
        for (var i = 0; i < Levels; i++)
        {
            sum += Vacancies[i];
            result[i] = sum;
        }
        return result;
    }

    /// <summary>
    /// 失業者が多いのに近くに空き職場が無い地区を、困っている度合いの大きい順に返す。
    /// </summary>
    /// <param name="limit">返す最大件数。</param>
    /// <param name="minUnemployed">この人数に満たない地区は対象にしない（小さな地区の誤差を除く）。</param>
    public IReadOnlyList<JobHotspot> Hotspots(int limit, long minUnemployed)
    {
        var withJobs = m_Cells.Where(p => p.Value.Vacancies > 0).ToList();
        return m_Cells
            .Where(p => p.Value.Unemployed >= minUnemployed && p.Value.Unemployed > p.Value.Vacancies)
            .OrderByDescending(p => p.Value.Unemployed - p.Value.Vacancies)
            .Take(limit)
            .Select(p =>
            {
                var c = p.Value;
                var reach = DistanceToGather(p.Key, (c.Unemployed - c.Vacancies + 1) / 2, withJobs);
                return new JobHotspot(p.Key.X, p.Key.Z, c.Unemployed, c.Vacancies, reach, c.CenterX, c.CenterY, c.CenterZ);
            })
            .ToList();
    }

    /// <summary>
    /// ほかの地区の空き職場を近い順に足していき、合計が <paramref name="target"/> に届く距離（m、地区の中心どうし）。
    /// 市内の空きを全部足しても届かなければ null。
    /// </summary>
    /// <remarks>
    /// 「いちばん近い空きのある地区」だと、隣の地区に空きが 1 件あるだけで近く見えてしまう。
    /// そこで不足分の半分を埋められる空きがどこまで行けば集まるかで、職の遠さを表す。
    /// 空きはほかの地区の失業者と取り合いになるので、実際にはこの距離より遠くまで通うことになる（控えめな見積もり）。
    /// </remarks>
    private static double? DistanceToGather((int X, int Z) from, long target, List<KeyValuePair<(int X, int Z), Cell>> withJobs)
    {
        if (target <= 0)
        {
            return 0;
        }
        long sum = 0;
        foreach (var (distance, vacancies) in withJobs
            .Where(o => o.Key != from)
            .Select(o =>
            {
                var dx = (o.Key.X - from.X) * (double)CellSize;
                var dz = (o.Key.Z - from.Z) * (double)CellSize;
                return (Math.Sqrt(dx * dx + dz * dz), o.Value.Vacancies);
            })
            .OrderBy(o => o.Item1))
        {
            sum += vacancies;
            if (sum >= target)
            {
                return distance;
            }
        }
        return null;
    }

    private static int Clamp(int level) => level < 0 ? 0 : level >= Levels ? Levels - 1 : level;

    private Cell CellAt(float x, float z)
    {
        var key = ((int)Math.Floor(x / CellSize), (int)Math.Floor(z / CellSize));
        if (!m_Cells.TryGetValue(key, out var cell))
        {
            cell = new Cell();
            m_Cells[key] = cell;
        }
        return cell;
    }

    private sealed class Cell
    {
        private double m_SumX;
        private double m_SumY;
        private double m_SumZ;
        private long m_Points;

        public long Unemployed;
        public long Vacancies;

        // カメラを移す先は、地区の中心ではなく人や職場の平均位置にする（海や山の上を避けるため）。
        public float CenterX => m_Points == 0 ? 0f : (float)(m_SumX / m_Points);

        public float CenterY => m_Points == 0 ? 0f : (float)(m_SumY / m_Points);

        public float CenterZ => m_Points == 0 ? 0f : (float)(m_SumZ / m_Points);

        public void AddPoint(float x, float y, float z)
        {
            m_SumX += x;
            m_SumY += y;
            m_SumZ += z;
            m_Points++;
        }
    }
}

/// <summary>失業者が多いのに職場が足りない地区。</summary>
public sealed class JobHotspot
{
    public JobHotspot(int cellX, int cellZ, long unemployed, long vacancies, double? jobsReachMeters, float x, float y, float z)
    {
        CellX = cellX;
        CellZ = cellZ;
        Unemployed = unemployed;
        Vacancies = vacancies;
        JobsReachMeters = jobsReachMeters;
        X = x;
        Y = y;
        Z = z;
    }

    public int CellX { get; }

    public int CellZ { get; }

    public long Unemployed { get; }

    public long Vacancies { get; }

    /// <summary>
    /// 不足分（失業者 − 地区内の空き）の半分を埋める空き職場が集まる距離（m）。市内の空きを全部足しても届かなければ null。
    /// </summary>
    public double? JobsReachMeters { get; }

    /// <summary>カメラを移す先（地区内の人と職場の平均位置）。</summary>
    public float X { get; }

    public float Y { get; }

    public float Z { get; }
}
