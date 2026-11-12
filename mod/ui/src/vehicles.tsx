// 「取り残し車両」タブ。C# 側 StrandedVehicleSystem の "vehicles"（StrandedVehicles.ToJson の JSON 文字列）を表示し、
// 探す・見に行く・選んで消すを C# 側のトリガーへ送る。
import { React, api } from "./game";
import { GROUP, colors, fmt, s } from "./style";

const vehicles$ = api.bindValue<string>(GROUP, "vehicles", "{}");

type Reason = "laneGone" | "notOnLane" | "ownerGone" | "noRoute" | "stuck";

interface VehicleGroup {
  key: string;
  reason: Reason;
  kind: "car" | "train" | "other";
  count: number;
  parked: number;
  hidden: number;
}

interface VehicleReport {
  state?: "idle" | "ready" | "deleting";
  scanned?: number;
  total?: number;
  pending?: number;
  lastDeleted?: number | null;
  groups?: VehicleGroup[];
}

const REASON_LABELS: Record<Reason, { title: string; detail: string }> = {
  laneGone: { title: "車線が無くなっている", detail: "撤去した道路・線路・駐車場の上に残っています。" },
  notOnLane: { title: "交通から外れている", detail: "車線は残っていますが、車線の上の物として扱われていません。" },
  ownerGone: { title: "持ち主がいない", detail: "持ち主の世帯や運転していた人がいなくなったまま残っています。" },
  noRoute: { title: "路線に属さない列車", detail: "どの路線にも組み込まれず、車庫へも戻らないまま走行中の扱いです。" },
  stuck: { title: "長く動いていない（参考）", detail: "渋滞や停車中のこともあります。見に行って確かめてから消してください。" },
};

const KIND_LABELS = { car: "車", train: "列車", other: "その他" };

function parse(text: string): VehicleReport {
  try {
    return JSON.parse(text) as VehicleReport;
  } catch {
    return {};
  }
}

export function VehiclesTab() {
  const text = api.useValue(vehicles$);
  const report = React.useMemo(() => parse(text), [text]);
  const groups = report.groups ?? [];
  const [selected, setSelected] = React.useState<Record<string, boolean>>({});
  const [cursor, setCursor] = React.useState<Record<string, number>>({});
  const [confirming, setConfirming] = React.useState(false);

  const chosen = groups.filter((g) => selected[g.key]);
  const chosenCount = chosen.reduce((n, g) => n + g.count, 0);

  const jump = (g: VehicleGroup) => {
    const i = cursor[g.key] ?? 0;
    api.trigger(GROUP, "jumpVehicle", g.key, i);
    setCursor({ ...cursor, [g.key]: (i + 1) % Math.max(1, g.count) });
  };

  const remove = () => {
    if (!confirming) {
      setConfirming(true);
      return;
    }
    api.trigger(GROUP, "deleteVehicles", chosen.map((g) => g.key).join("\n"));
    setConfirming(false);
    setSelected({});
  };

  return (
    <div>
      <div style={s.row}>
        <div style={s.heading}>取り残された列車・車</div>
        <button style={s.smallButton} onClick={() => api.trigger(GROUP, "scanVehicles")}>
          {report.state === "idle" ? "探す" : "探し直す"}
        </button>
      </div>
      <div style={s.sub}>
        {"線路や駐車場の撤去などで取り残され、残り続けている車両です。持ち主がいない車などは、一時的な状態と見分けるため、2 分ほど続いたものだけを表示します。"}
      </div>

      {report.state !== "idle" && report.scanned !== undefined && (
        <div style={{ ...s.sub, marginTop: "4rem" }}>{`調べた車両: ${fmt(report.scanned)} 台`}</div>
      )}
      {(report.pending ?? 0) > 0 && (
        <div style={{ marginTop: "4rem", color: colors.warn }}>
          {`ほかに ${fmt(report.pending)} 台を確かめ中です。ゲーム内で 2 分ほど進めてから「探し直す」を押してください。`}
        </div>
      )}
      {report.state === "deleting" && <div style={{ marginTop: "6rem" }}>{`${fmt(report.lastDeleted)} 台を削除しました。一覧を更新しています…`}</div>}
      {report.state === "ready" && groups.length === 0 && <div style={{ marginTop: "8rem" }}>見つかりませんでした。</div>}

      {groups.map((g) => (
        <div key={g.key} style={s.section}>
          <div style={s.row}>
            <div
              style={{ pointerEvents: "auto", display: "flex", flexDirection: "row", alignItems: "center" }}
              onClick={() => setSelected({ ...selected, [g.key]: !selected[g.key] })}
            >
              {/* ゲームのフォントに ☐☑ が無いので、枠と塗りでチェック欄を描く。 */}
              <div
                style={{
                  width: "12rem",
                  height: "12rem",
                  marginRight: "8rem",
                  border: "2rem solid white",
                  borderRadius: "2rem",
                  background: selected[g.key] ? colors.bar : "transparent",
                }}
              />
              <div>{`${REASON_LABELS[g.reason].title}（${KIND_LABELS[g.kind]}）`}</div>
            </div>
            <button style={s.smallButton} onClick={() => jump(g)}>
              見に行く
            </button>
          </div>
          <div style={s.sub}>{`${fmt(g.count)} 台（うち駐車中 ${fmt(g.parked)}）。${REASON_LABELS[g.reason].detail}`}</div>
        </div>
      ))}

      {chosenCount > 0 && (
        <div style={s.section}>
          <div style={{ color: confirming ? colors.warn : "white" }}>
            {confirming
              ? `本当に ${fmt(chosenCount)} 台を削除しますか？ 元に戻すには、保存せずに街を読み込み直してください。`
              : `${fmt(chosenCount)} 台を選んでいます。`}
          </div>
          <div style={{ ...s.row, justifyContent: "flex-start", marginTop: "6rem" }}>
            <button style={{ ...s.smallButton, background: confirming ? "rgba(255, 112, 67, 0.6)" : s.smallButton.background }} onClick={remove}>
              {confirming ? "削除する" : "選んだ車両を削除"}
            </button>
            {confirming && (
              <button style={{ ...s.smallButton, marginLeft: "8rem" }} onClick={() => setConfirming(false)}>
                やめる
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
