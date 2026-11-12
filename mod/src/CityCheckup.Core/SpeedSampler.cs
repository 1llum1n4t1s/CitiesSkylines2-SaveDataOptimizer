using System;

namespace CityCheckup.Core;

/// <summary>
/// 画面のフレームごとに「シミュレーションがなぜ設定速度より遅いか」の材料を集める。
/// </summary>
/// <remarks>
/// ゲーム（SimulationSystem）は次の 2 つの理由で、設定した速度より遅く進む。
/// 1. 経路探し待ち: 経路探しの結果が予定のフレームまでに届かないと、その 48 フレーム手前から速度を絞り、
///    予定のフレームに達すると止まって結果を待つ（PENDING_FRAMES_SPEED_FACTOR = 1/48）。
///    ここではゲームと同じ式で絞り具合を計算し、その時間の割合を「経路探し待ち」とする。
/// 2. シミュレーション処理の重さ: 1 フレームの処理時間（frameDuration）が、設定速度で必要な間隔
///    （1 / (60 × 速度) 秒）より長いと追いつけない。
/// 一時停止中（設定速度 0）の時間は数えない。
/// </remarks>
public sealed class SpeedSampler
{
    /// <summary>ゲームが速度を絞り始める、結果待ちまでの残りフレーム数。</summary>
    public const float PendingFramesForFullSpeed = 48f;

    private double m_RunningSeconds;
    private double m_ThrottledSeconds;
    private double m_SimFrameSecondsSum;
    private long m_SimFrameSamples;
    private double m_PendingSum;
    private long m_Samples;
    private long m_RenderFrames;
    private double m_SelectedSpeedTime;
    private double m_ActualSpeedTime;

    /// <summary>
    /// 経路探しの結果待ちで、ゲームがその瞬間の速度にかけている倍率（0〜1）。ゲームの SimulationSystem と同じ計算。
    /// </summary>
    /// <param name="pendingFrame">次に結果が必要なフレーム（uint.MaxValue なら待ちなし）。</param>
    /// <param name="frameIndex">今のシミュレーションのフレーム。</param>
    public static double PathfindSpeedFactor(uint pendingFrame, uint frameIndex)
    {
        if (pendingFrame == uint.MaxValue)
        {
            return 1;
        }
        var remaining = (long)pendingFrame - frameIndex - 1;
        return Math.Max(0, Math.Min(1, remaining / PendingFramesForFullSpeed));
    }

    /// <param name="realSeconds">前回からの実時間（秒）。</param>
    /// <param name="selectedSpeed">設定した速度（0 なら一時停止）。</param>
    /// <param name="actualSpeed">ゲームが表示している実際の速度（smoothSpeed）。</param>
    /// <param name="pathfindFactor"><see cref="PathfindSpeedFactor"/> の値。</param>
    /// <param name="simFrameSeconds">シミュレーション 1 フレームの処理時間（秒）。この画面フレームで進まなかったときは 0。</param>
    /// <param name="pendingRequests">結果を待っている経路探しの件数。</param>
    public void Add(double realSeconds, float selectedSpeed, float actualSpeed, double pathfindFactor, double simFrameSeconds, int pendingRequests)
    {
        m_RenderFrames++;
        if (selectedSpeed <= 0 || realSeconds <= 0)
        {
            return;
        }
        m_RunningSeconds += realSeconds;
        m_ThrottledSeconds += realSeconds * (1 - pathfindFactor);
        m_SelectedSpeedTime += realSeconds * selectedSpeed;
        m_ActualSpeedTime += realSeconds * actualSpeed;
        if (simFrameSeconds > 0)
        {
            m_SimFrameSecondsSum += simFrameSeconds;
            m_SimFrameSamples++;
        }
        m_PendingSum += pendingRequests;
        m_Samples++;
    }

    /// <summary>区間を締めて結果を返し、次の区間を始める。</summary>
    public SpeedWindow Close(double realSeconds)
    {
        var w = new SpeedWindow(
            runningSeconds: m_RunningSeconds,
            pathfindWaitShare: m_RunningSeconds > 0 ? m_ThrottledSeconds / m_RunningSeconds : double.NaN,
            simFrameMilliseconds: m_SimFrameSamples > 0 ? m_SimFrameSecondsSum / m_SimFrameSamples * 1000 : double.NaN,
            averageSelectedSpeed: m_RunningSeconds > 0 ? m_SelectedSpeedTime / m_RunningSeconds : double.NaN,
            averageActualSpeed: m_RunningSeconds > 0 ? m_ActualSpeedTime / m_RunningSeconds : double.NaN,
            averagePendingRequests: m_Samples > 0 ? m_PendingSum / m_Samples : double.NaN,
            framesPerSecond: realSeconds > 0 ? m_RenderFrames / realSeconds : double.NaN);
        Reset();
        return w;
    }

    public void Reset()
    {
        m_RunningSeconds = m_ThrottledSeconds = m_SimFrameSecondsSum = m_PendingSum = m_SelectedSpeedTime = m_ActualSpeedTime = 0;
        m_SimFrameSamples = m_Samples = m_RenderFrames = 0;
    }
}

/// <summary>締めた 1 区間の速度の内訳。</summary>
public sealed class SpeedWindow
{
    public SpeedWindow(double runningSeconds, double pathfindWaitShare, double simFrameMilliseconds, double averageSelectedSpeed, double averageActualSpeed, double averagePendingRequests, double framesPerSecond)
    {
        RunningSeconds = runningSeconds;
        PathfindWaitShare = pathfindWaitShare;
        SimFrameMilliseconds = simFrameMilliseconds;
        AverageSelectedSpeed = averageSelectedSpeed;
        AverageActualSpeed = averageActualSpeed;
        AveragePendingRequests = averagePendingRequests;
        FramesPerSecond = framesPerSecond;
    }

    /// <summary>一時停止していなかった時間（秒）。</summary>
    public double RunningSeconds { get; }

    /// <summary>経路探しの結果待ちで速度を絞られていた時間の割合（0〜1、絞った強さで重み付け）。</summary>
    public double PathfindWaitShare { get; }

    /// <summary>シミュレーション 1 フレームの平均処理時間（ミリ秒）。</summary>
    public double SimFrameMilliseconds { get; }

    public double AverageSelectedSpeed { get; }

    public double AverageActualSpeed { get; }

    /// <summary>結果を待っている経路探しの平均件数。</summary>
    public double AveragePendingRequests { get; }

    /// <summary>画面の 1 秒あたりのフレーム数。</summary>
    public double FramesPerSecond { get; }

    /// <summary>設定速度で必要な、シミュレーション 1 フレームあたりの時間の上限（ミリ秒）。</summary>
    public double SimFrameBudgetMilliseconds => AverageSelectedSpeed > 0 ? 1000 / (60 * AverageSelectedSpeed) : double.NaN;
}
