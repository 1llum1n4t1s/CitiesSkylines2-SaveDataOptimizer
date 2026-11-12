using System.IO;

namespace CityCheckup;

/// <summary>
/// 動作確認用の目印ファイル。Mods\CityCheckup\ に置いたときだけ有効になる。
/// </summary>
/// <remarks>
/// 画面を見たり操作したりできない検証機（リモートのゲーム PC）で、UI 操作の代わりに処理を自動で動かし、
/// 結果を Player.log と画面写真に残すために使う。通常の利用者の環境には置かれない。
/// - debug-report : 負荷分析を開いた状態で始め、集計結果をログに書き、画面を保存する
/// - debug-missing: 欠落アセットを探して一覧をログに書き、最初の 1 か所へカメラを移して画面を保存する
/// - debug-delete : debug-missing に加えて、消してよいものを全部消し、消した後の状態をログに書く
/// </remarks>
internal static class DebugMarkers
{
    public static string Folder => Path.Combine(UnityEngine.Application.persistentDataPath, "Mods", "CityCheckup");

    public static bool Has(string name) => File.Exists(Path.Combine(Folder, name));

    public static string PathOf(string name) => Path.Combine(Folder, name);
}
