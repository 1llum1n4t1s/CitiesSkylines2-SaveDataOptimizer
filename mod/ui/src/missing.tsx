// 「欠落アセット」タブ。C# 側 MissingAssetSystem の "missing"（MissingAssets.ToJson の JSON 文字列）を表示し、
// 探す・見に行く・選んで消すを C# 側のトリガーへ送る。
import { React, api } from "./game";
import { useTexts } from "./i18n";
import { GROUP, colors, s } from "./style";

const missing$ = api.bindValue<string>(GROUP, "missing", "{}");

interface MissingGroup {
  key: string;
  type: string;
  name: string;
  kind: "building" | "object" | "net" | "area";
  count: number;
  deletable: number;
  positions: [number, number, number][];
}

interface MissingReport {
  state?: "idle" | "ready" | "deleting";
  total?: number;
  lastDeleted?: number | null;
  groups?: MissingGroup[];
}

function parse(text: string): MissingReport {
  try {
    return JSON.parse(text) as MissingReport;
  } catch {
    return {};
  }
}

export function MissingTab() {
  const t = useTexts();
  const text = api.useValue(missing$);
  const report = React.useMemo(() => parse(text), [text]);
  const groups = report.groups ?? [];
  // 選んだグループ（key）と、「見に行く」で次に見る順番。
  const [selected, setSelected] = React.useState<Record<string, boolean>>({});
  const [cursor, setCursor] = React.useState<Record<string, number>>({});
  // 削除は 2 段階（1 回目で確認表示、2 回目で実行）にして押し間違いを防ぐ。
  const [confirming, setConfirming] = React.useState(false);

  const chosen = groups.filter((g) => selected[g.key] && g.deletable > 0);
  const chosenCount = chosen.reduce((n, g) => n + g.deletable, 0);

  const jump = (g: MissingGroup) => {
    const i = cursor[g.key] ?? 0;
    api.trigger(GROUP, "jumpMissing", g.key, i);
    setCursor({ ...cursor, [g.key]: (i + 1) % Math.max(1, g.count) });
  };

  const remove = () => {
    if (!confirming) {
      setConfirming(true);
      return;
    }
    api.trigger(GROUP, "deleteMissing", chosen.map((g) => g.key).join("\n"));
    setConfirming(false);
    setSelected({});
  };

  return (
    <div>
      <div style={s.row}>
        <div style={s.heading}>{t.missingHeading}</div>
        <button style={s.smallButton} onClick={() => api.trigger(GROUP, "scanMissing")}>
          {report.state === "idle" ? t.scan : t.rescan}
        </button>
      </div>
      <div style={s.sub}>
        {t.missingNote}
      </div>

      {report.state === "deleting" && <div style={{ marginTop: "6rem" }}>{t.deletedRefreshing(report.lastDeleted)}</div>}
      {report.state === "ready" && report.lastDeleted !== null && report.lastDeleted !== undefined && (
        <div style={{ marginTop: "6rem" }}>{t.deletedJustNow(report.lastDeleted)}</div>
      )}
      {report.state === "ready" && groups.length === 0 && <div style={{ marginTop: "8rem" }}>{t.notFound}</div>}

      {groups.map((g) => (
        <div key={g.key} style={s.section}>
          <div style={s.row}>
            <div
              style={{ pointerEvents: "auto", display: "flex", flexDirection: "row", alignItems: "center", opacity: g.deletable > 0 ? 1 : 0.5 }}
              onClick={() => g.deletable > 0 && setSelected({ ...selected, [g.key]: !selected[g.key] })}
            >
              {/* ゲームのフォントに ☐☑ が無いので、枠と塗りでチェック欄を描く。 */}
              <div
                style={{
                  width: "12rem",
                  height: "12rem",
                  marginRight: "8rem",
                  border: `2rem solid ${g.deletable > 0 ? "white" : colors.sub}`,
                  borderRadius: "2rem",
                  background: selected[g.key] ? colors.bar : "transparent",
                }}
              />
              <div>{g.name}</div>
            </div>
            <button style={s.smallButton} onClick={() => jump(g)}>
              {t.goSee}
            </button>
          </div>
          <div style={s.sub}>
            {t.groupDetail(t.kinds[g.kind] ?? g.kind, g.count, g.deletable)}
          </div>
        </div>
      ))}

      {chosenCount > 0 && (
        <div style={s.section}>
          <div style={{ color: confirming ? colors.warn : "white" }}>
            {confirming
              ? t.confirmDelete(chosenCount)
              : t.selectedCount(chosenCount)}
          </div>
          <div style={{ ...s.row, justifyContent: "flex-start", marginTop: "6rem" }}>
            <button style={{ ...s.smallButton, background: confirming ? "rgba(255, 112, 67, 0.6)" : s.smallButton.background }} onClick={remove}>
              {confirming ? t.deleteNow : t.deleteSelected}
            </button>
            {confirming && (
              <button style={{ ...s.smallButton, marginLeft: "8rem" }} onClick={() => setConfirming(false)}>
                {t.cancel}
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
