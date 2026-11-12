using Game.Rendering;
using Vector3 = UnityEngine.Vector3;

namespace SaveDataOptimizer.Ui;

/// <summary>一覧から選ばれた位置へカメラを移す（各パネルで共通）。</summary>
internal static class CameraJump
{
    /// <summary>
    /// 移動後のカメラの最小距離（m）。注視点だけを移すと、直前の距離のまま建物の壁際などに寄りすぎることがあるため、
    /// これより近ければ引いて周りが見えるようにする。
    /// </summary>
    private const float MinZoom = 250f;

    /// <summary>今使われているカメラ操作（通常は遊ぶときのカメラ）の注視点を移す。視点の向きは変えない。</summary>
    public static void To(CameraUpdateSystem camera, float x, float y, float z)
    {
        var controller = camera.activeCameraController;
        if (controller is null)
        {
            return;
        }
        controller.pivot = new Vector3(x, y, z);
        if (controller.zoom < MinZoom)
        {
            controller.zoom = MinZoom;
        }
    }
}
