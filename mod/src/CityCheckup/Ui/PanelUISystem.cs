using System.Diagnostics;
using Colossal.Serialization.Entities;
using Colossal.UI.Binding;
using Game;
using Game.Rendering;
using Game.Simulation;
using Game.UI;
using CityCheckup.Analysis;
using CityCheckup.Core;
using Debug = UnityEngine.Debug;

namespace CityCheckup.Ui;

/// <summary>
/// 負荷分析パネルと C# をつなぐバインディングを持ち、パネルを開いている間だけ集計を動かす。
/// </summary>
/// <remarks>
/// UI（ui/src）とはグループ名 <see cref="Group"/> の下の名前で対応する。名前を変えるときは両方を直すこと。
/// 表示データは Core の <see cref="LoadReport"/> で JSON 文字列にし、1 本の値で渡す（UI 側で JSON.parse）。
/// 集計は重いので、パネルを閉じている間は速度の表示以外なにもしない（分析ツール自身が負荷にならないように）。
/// </remarks>
public sealed partial class PanelUISystem : UISystemBase
{
    public const string Group = "cityCheckup";

    // 表示の更新間隔（実時間の秒）。速度の表示だけはこの間隔で更新する。
    private const float RefreshSeconds = 1f;

    // 職・世帯の集計と、経路探しの区間を締める間隔（実時間の秒）。集計は数十ミリ秒かかるので間を空ける。
    private const float ScanSeconds = 10f;

    private ValueBinding<string> m_Status = null!;
    private SimulationSystem m_Simulation = null!;
    private CameraUpdateSystem m_Camera = null!;
    private CityScanner m_Scanner = null!;
    private LoadProbeSystem m_Probe = null!;
    private readonly LoadReport m_Report = new();
    private bool m_Open;
    private bool m_InGame;
    private bool m_DebugReport;
    private string m_DebugShotPath = string.Empty;
    private int m_DebugScans;
    private float m_NextRefresh;
    private float m_NextScan;

    protected override void OnCreate()
    {
        base.OnCreate();
        m_Simulation = World.GetOrCreateSystemManaged<SimulationSystem>();
        m_Camera = World.GetOrCreateSystemManaged<CameraUpdateSystem>();
        m_Probe = World.GetOrCreateSystemManaged<LoadProbeSystem>();
        m_Scanner = new CityScanner(
            EntityManager,
            GetEntityQuery(CityScanner.JobSeekersDesc()),
            GetEntityQuery(CityScanner.WorkplacesDesc()),
            GetEntityQuery(CityScanner.CitizensDesc()),
            GetEntityQuery(CityScanner.HomelessDesc()),
            GetEntityQuery(CityScanner.MovingAwayDesc()));

        m_Status = new ValueBinding<string>(Group, "status", "{}");
        AddBinding(m_Status);
        AddBinding(new TriggerBinding<bool>(Group, "setOpen", OnSetOpen));
        AddBinding(new TriggerBinding<float, float, float>(Group, "jumpTo", OnJumpTo));
        // UI が表示されたことを Player.log で確かめるための合図。画面を見られない環境での動作確認に使う。
        AddBinding(new TriggerBinding<string>(Group, "uiMounted", OnUiMounted));

        // 動作確認用: Mods\CityCheckup\debug-report があれば、パネルを開いた状態で始めて集計を動かし、
        // 集計のたびに結果の JSON を Player.log へ書き、少し経ったら画面を debug-shot.png に保存する
        // （画面を見たり操作したりできない検証機で、中身と見た目を確かめるため）。
        m_DebugReport = DebugMarkers.Has("debug-report");
        m_DebugShotPath = DebugMarkers.PathOf("debug-shot.png");
        var debugMissing = DebugMarkers.Has("debug-missing") || DebugMarkers.Has("debug-delete");
        AddBinding(new ValueBinding<bool>(Group, "debugOpen", m_DebugReport || debugMissing));
        // 動作確認で最初に開くタブ。
        AddBinding(new ValueBinding<string>(Group, "debugTab", debugMissing ? "missing" : "load"));
        if (m_DebugReport)
        {
            OnSetOpen(true);
            Debug.Log($"{Mod.LogPrefix} debug-report enabled");
        }
    }

    /// <summary>街を遊んでいる間だけ動かす（メインメニューやエディターでは集計しない）。</summary>
    public override GameMode gameMode => GameMode.Game;

    protected override void OnGamePreload(Purpose purpose, GameMode mode)
    {
        base.OnGamePreload(purpose, mode);
        // 読み込み中は街のデータがそろっていないので数えない（読み込み画面の間も UI システム自体は動く）。
        m_InGame = false;
    }

    protected override void OnGameLoadingComplete(Purpose purpose, GameMode mode)
    {
        base.OnGameLoadingComplete(purpose, mode);
        m_InGame = mode == GameMode.Game;
        // 読み込み前に開いていた場合も、読み込み完了の直後に数え直す。
        m_NextScan = 0f;
        m_Probe.Begin();
        m_Report.Pathfind = null;
        m_Report.Speed = null;
        m_Report.Jobs = null;
    }

    protected override void OnUpdate()
    {
        base.OnUpdate();
        if (!m_InGame)
        {
            return;
        }
        // SystemBase.Time（シミュレーション時間）と紛らわしいので UnityEngine の実時間を明示する。
        var now = UnityEngine.Time.realtimeSinceStartup;
        if (now < m_NextRefresh)
        {
            return;
        }
        m_NextRefresh = now + RefreshSeconds;
        // 動作確認用: debug-speed3 があれば、速度を ×3 に保つ（遅れの原因の切り分けを確かめるため）。
        // ゲームは読み込み完了の後に速度を設定し直すので、読み込み完了時ではなくここで毎秒そろえる。
        if (m_DebugReport && m_Simulation.selectedSpeed > 0 && m_Simulation.selectedSpeed != 3f && DebugMarkers.Has("debug-speed3"))
        {
            m_Simulation.selectedSpeed = 3f;
            Debug.Log($"{Mod.LogPrefix} debug speed x3");
        }
        m_Report.SelectedSpeed = m_Simulation.selectedSpeed;
        m_Report.ActualSpeed = m_Simulation.smoothSpeed;

        if (m_Open && now >= m_NextScan)
        {
            m_NextScan = now + ScanSeconds;
            (m_Report.Pathfind, m_Report.Speed) = m_Probe.Close();
            var watch = Stopwatch.StartNew();
            m_Scanner.Fill(m_Report);
            m_Report.ScanMilliseconds = watch.Elapsed.TotalMilliseconds;
            if (m_DebugReport)
            {
                Debug.Log($"{Mod.LogPrefix} report {m_Report.ToJson(true)}");
                // 経路探しの集計が出そろった 2 回目以降の集計で 1 度だけ画面を保存する。
                m_DebugScans++;
                if (m_DebugScans == 3)
                {
                    UnityEngine.ScreenCapture.CaptureScreenshot(m_DebugShotPath);
                    Debug.Log($"{Mod.LogPrefix} screenshot requested");
                }
            }
        }
        m_Status.Update(m_Report.ToJson(m_Open));
    }

    private void OnSetOpen(bool open)
    {
        // 動作確認用の自動集計中は、UI のパネルを閉じても集計を止めない。
        if (m_DebugReport && !open)
        {
            return;
        }
        if (open == m_Open)
        {
            return;
        }
        m_Open = open;
        m_Probe.Collecting = open;
        if (open)
        {
            // 開いた直後に職・世帯を数え、経路探しと速度の内訳は次の区間（ScanSeconds 後）から表示する。
            m_Probe.Begin();
            m_NextScan = 0f;
            m_NextRefresh = 0f;
            m_Report.Pathfind = null;
            m_Report.Speed = null;
        }
    }

    /// <summary>地区の一覧から選ばれた位置へカメラを移す。</summary>
    private void OnJumpTo(float x, float y, float z) => CameraJump.To(m_Camera, x, y, z);

    private void OnUiMounted(string detail)
    {
        Debug.Log($"{Mod.LogPrefix} ui mounted {detail}");
    }
}
