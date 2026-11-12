// 「負荷分析」タブ。C# 側 PanelUISystem の "status"（LoadReport.ToJson の JSON 文字列）を表示する。
import { React, api } from "./game";
import { GROUP, colors, fmt, pct, s } from "./style";

const status$ = api.bindValue<string>(GROUP, "status", "{}");

// LoadReport.ToJson の形。値が null のときは「まだ数えていない」か「測れない」。
interface Report {
  collecting?: boolean;
  speed?: { selected: number | null; actual: number | null };
  citizens?: number;
  households?: { homeless: number; movingAway: number };
  scanMs?: number | null;
  pathfind?: {
    seconds: number;
    frames: number;
    total: number;
    perFrame: number | null;
    sources: { key: string; count: number; perFrame: number | null; share: number | null }[];
  } | null;
  jobs?: {
    unemployed: number[];
    vacancies: number[];
    reachable: number[];
    commuters: number;
    // reachKm: 足りない職の半分を埋める空きが集まる距離。null は市内の空きを全部足しても届かない。
    hotspots: { unemployed: number; vacancies: number; reachKm: number | null; x: number; y: number; z: number }[];
  } | null;
  findings?: { level: "warning" | "info"; text: string }[];
}

// 経路探しの依頼元（Core の PathfindSources.Key）の表示名。
const SOURCE_LABELS: Record<string, string> = {
  jobSearch: "失業者の職探し",
  jobSwitch: "転職先探し",
  shopping: "市民の買い物",
  residentRepath: "歩行者の経路の引き直し",
  trips: "外出・レジャー",
  school: "学校探し",
  home: "住まい探し",
  serviceDispatch: "サービス車両の配車",
  vehicleRepath: "車両の経路の引き直し",
  other: "その他",
};

// 学歴の段階 0〜4。
const LEVEL_LABELS = ["教育なし", "初等教育", "中等教育", "高等教育", "大学院"];

function parseReport(text: string): Report {
  try {
    return JSON.parse(text) as Report;
  } catch {
    // 壊れた値で UI 全体を落とさない。次の更新で直る。
    return {};
  }
}


function Row(props: { label: any; value: any }) {
  return (
    <div style={s.row}>
      <div>{props.label}</div>
      <div>{props.value}</div>
    </div>
  );
}

function Findings({ report }: { report: Report }) {
  const items = report.findings ?? [];
  return (
    <div>
      <div style={s.heading}>気づいたこと</div>
      {items.map((f, i) => (
        // 文字列は 1 つにまとめて渡す（Gameface では隣り合う文字列が別の行に分かれることがあるため）。
        <div key={i} style={{ marginTop: "3rem", color: f.level === "warning" ? colors.warn : "white" }}>
          {`${f.level === "warning" ? "⚠ " : ""}${f.text}`}
        </div>
      ))}
    </div>
  );
}

function Speed({ report }: { report: Report }) {
  const sp = report.speed;
  return (
    <div style={s.section}>
      <div style={s.heading}>シミュレーションの速度</div>
      <Row label="設定した速度" value={`×${fmt(sp?.selected, 1)}`} />
      <Row label="実際の速度" value={`×${fmt(sp?.actual, 2)}`} />
      <Row label="人のデータ（観光客・通勤者を含む）" value={`${fmt(report.citizens)} 人`} />
    </div>
  );
}

function Pathfind({ report }: { report: Report }) {
  const p = report.pathfind;
  return (
    <div style={s.section}>
      <div style={s.heading}>経路探しの依頼</div>
      {!report.collecting && <div style={s.sub}>このゲームの版では数えられません。</div>}
      {report.collecting && !p && <div style={s.sub}>数えています（10 秒ほどで表示します）…</div>}
      {p && (
        <div>
          <Row label="1 フレームあたり" value={`${fmt(p.perFrame, 1)} 件`} />
          {p.sources.map((src) => (
            <div key={src.key} style={{ marginTop: "4rem" }}>
              <div style={s.row}>
                <div>{SOURCE_LABELS[src.key] ?? src.key}</div>
                <div>{`${pct(src.share)}（${fmt(src.perFrame, 1)} 件）`}</div>
              </div>
              <div style={{ height: "4rem", background: colors.line, marginTop: "2rem" }}>
                <div style={{ height: "4rem", width: `${Math.round((src.share ?? 0) * 100)}%`, background: colors.bar }} />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function Jobs({ report }: { report: Report }) {
  const j = report.jobs;
  if (!j) {
    return (
      <div style={s.section}>
        <div style={s.heading}>職と失業</div>
        <div style={s.sub}>数えています…</div>
      </div>
    );
  }
  return (
    <div style={s.section}>
      <div style={s.heading}>職と失業</div>
      <div style={{ ...s.row, ...s.sub }}>
        <div style={{ width: "34%" }}>学歴</div>
        <div style={{ width: "33%", textAlign: "right" }}>職を探す人</div>
        <div style={{ width: "33%", textAlign: "right" }}>就ける空き</div>
      </div>
      {LEVEL_LABELS.map((label, i) => (
        <div key={i} style={s.row}>
          <div style={{ width: "34%" }}>{label}</div>
          <div style={{ width: "33%", textAlign: "right" }}>{fmt(j.unemployed[i])}</div>
          <div style={{ width: "33%", textAlign: "right", color: j.reachable[i] < j.unemployed[i] ? colors.warn : "white" }}>
            {fmt(j.reachable[i])}
          </div>
        </div>
      ))}
      <div style={{ ...s.sub, marginTop: "4rem" }}>
        {`「就ける空き」は、その学歴以下で就ける空き職場の合計です。市外からの通勤者で職の無い人: ${fmt(j.commuters)} 人`}
      </div>
      {j.hotspots.length > 0 && (
        <div style={{ marginTop: "8rem" }}>
          <div style={s.heading}>職の足りない地区</div>
          {j.hotspots.map((h, i) => (
            <div key={i} style={s.row}>
              <div>
                <div>{`失業 ${fmt(h.unemployed)} 人 / 空き ${fmt(h.vacancies)}`}</div>
                <div style={s.sub}>
                  {h.reachKm === null ? "市内の空きを全部合わせても足りません" : `足りない職の半分が集まるのは約 ${fmt(h.reachKm, 1)} km 先`}
                </div>
              </div>
              <button style={s.smallButton} onClick={() => api.trigger(GROUP, "jumpTo", h.x, h.y, h.z)}>
                見に行く
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function Households({ report }: { report: Report }) {
  const h = report.households;
  return (
    <div style={s.section}>
      <div style={s.heading}>世帯</div>
      <Row label="ホームレスの世帯" value={fmt(h?.homeless)} />
      <Row label="転出中の世帯" value={fmt(h?.movingAway)} />
      <div style={{ ...s.sub, marginTop: "8rem" }}>{`集計にかかった時間: ${fmt(report.scanMs, 0)} ms（10 秒ごと）`}</div>
    </div>
  );
}


/** C# から負荷の値が届いたか（UI が表示できたことの確認に使う）。 */
export function loadReady(): boolean {
  const text = api.useValue(status$);
  return text !== "{}";
}

/** 「負荷分析」タブの中身。 */
export function LoadTab() {
  const statusText = api.useValue(status$);
  const report = React.useMemo(() => parseReport(statusText), [statusText]);
  return (
    <div>
      <Findings report={report} />
      <Speed report={report} />
      <Pathfind report={report} />
      <Jobs report={report} />
      <Households report={report} />
    </div>
  );
}