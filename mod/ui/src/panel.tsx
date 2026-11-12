// 「負荷分析」タブ。C# 側 PanelUISystem の "status"（LoadReport.ToJson の JSON 文字列）を表示する。
import { React, api } from "./game";
import { useTexts } from "./i18n";
import { GROUP, colors, fmt, pct, s } from "./style";

const status$ = api.bindValue<string>(GROUP, "status", "{}");

// LoadReport.ToJson の形。値が null のときは「まだ数えていない」か「測れない」。
interface Report {
  collecting?: boolean;
  speed?: {
    selected: number | null;
    actual: number | null;
    // 直近の区間（約 10 秒）の内訳。pathfindWait は経路探し待ちで速度を落とした時間の割合。
    window: { pathfindWait: number | null; simFrameMs: number | null; budgetMs: number | null; pending: number | null; fps: number | null } | null;
  };
  citizens?: number;
  households?: { homeless: number; movingAway: number };
  scanMs?: number | null;
  pathfind?: {
    seconds: number;
    frames: number;
    total: number;
    perFrame: number | null;
    // workShare: 経路探しの作業量（探索した道の量）に占める割合。successRate: 行き先が見つかった割合。
    sources: { key: string; count: number; perFrame: number | null; workShare: number | null; successRate: number | null }[];
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


function parseReport(text: string): Report {
  try {
    return JSON.parse(text) as Report;
  } catch {
    // 壊れた値で UI 全体を落とさない。次の更新で直る。
    return {};
  }
}

type T = ReturnType<typeof useTexts>;

function Row(props: { label: any; value: any }) {
  return (
    <div style={s.row}>
      <div>{props.label}</div>
      <div>{props.value}</div>
    </div>
  );
}

function Findings({ report, t }: { report: Report; t: T }) {
  const items = report.findings ?? [];
  return (
    <div>
      <div style={s.heading}>{t.findings}</div>
      {items.map((f, i) => (
        // 文字列は 1 つにまとめて渡す（Gameface では隣り合う文字列が別の行に分かれることがあるため）。
        <div key={i} style={{ marginTop: "3rem", color: f.level === "warning" ? colors.warn : "white" }}>
          {`${f.level === "warning" ? "⚠ " : ""}${f.text}`}
        </div>
      ))}
    </div>
  );
}

function Speed({ report, t }: { report: Report; t: T }) {
  const sp = report.speed;
  return (
    <div style={s.section}>
      <div style={s.heading}>{t.speedHeading}</div>
      <Row label={t.selectedSpeed} value={`×${fmt(sp?.selected, 1)}`} />
      <Row label={t.actualSpeed} value={`×${fmt(sp?.actual, 2)}`} />
      {sp?.window && (
        <div>
          <Row label={t.pathfindWait} value={pct(sp.window.pathfindWait)} />
          <Row label={t.simFrame} value={t.simFrameValue(sp.window.simFrameMs, sp.window.budgetMs)} />
          <Row label={t.pending} value={t.count(sp.window.pending)} />
          <Row label={t.fps} value={`${fmt(sp.window.fps, 0)} fps`} />
        </div>
      )}
      <Row label={t.citizens} value={t.people(report.citizens)} />
    </div>
  );
}

function Pathfind({ report, t }: { report: Report; t: T }) {
  const p = report.pathfind;
  return (
    <div style={s.section}>
      <div style={s.heading}>{t.pathfindHeading}</div>
      {!p && <div style={s.sub}>{t.counting10s}</div>}
      {p && (
        <div>
          <Row label={t.requestsPerFrame} value={t.count(p.perFrame, 1)} />
          <div style={{ ...s.sub, marginTop: "2rem" }}>{t.workNote}</div>
          {p.sources.map((src) => (
            <div key={src.key} style={{ marginTop: "4rem" }}>
              <div style={s.row}>
                <div>{t.sources[src.key] ?? src.key}</div>
                <div>{t.sourceValue(pct(src.workShare), src.perFrame, pct(src.successRate))}</div>
              </div>
              <div style={{ height: "4rem", background: colors.line, marginTop: "2rem" }}>
                <div style={{ height: "4rem", width: `${Math.round((src.workShare ?? 0) * 100)}%`, background: colors.bar }} />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function Jobs({ report, t }: { report: Report; t: T }) {
  const j = report.jobs;
  if (!j) {
    return (
      <div style={s.section}>
        <div style={s.heading}>{t.jobsHeading}</div>
        <div style={s.sub}>{t.counting}</div>
      </div>
    );
  }
  return (
    <div style={s.section}>
      <div style={s.heading}>{t.jobsHeading}</div>
      <div style={{ ...s.row, ...s.sub }}>
        <div style={{ width: "34%" }}>{t.education}</div>
        <div style={{ width: "33%", textAlign: "right" }}>{t.seekers}</div>
        <div style={{ width: "33%", textAlign: "right" }}>{t.reachable}</div>
      </div>
      {t.levels.map((label, i) => (
        <div key={i} style={s.row}>
          <div style={{ width: "34%" }}>{label}</div>
          <div style={{ width: "33%", textAlign: "right" }}>{fmt(j.unemployed[i])}</div>
          <div style={{ width: "33%", textAlign: "right", color: j.reachable[i] < j.unemployed[i] ? colors.warn : "white" }}>
            {fmt(j.reachable[i])}
          </div>
        </div>
      ))}
      <div style={{ ...s.sub, marginTop: "4rem" }}>{t.reachableNote(j.commuters)}</div>
      {j.hotspots.length > 0 && (
        <div style={{ marginTop: "8rem" }}>
          <div style={s.heading}>{t.hotspotsHeading}</div>
          {j.hotspots.map((h, i) => (
            <div key={i} style={s.row}>
              <div>
                <div>{t.hotspot(h.unemployed, h.vacancies)}</div>
                <div style={s.sub}>{t.hotspotReach(h.reachKm)}</div>
              </div>
              <button style={s.smallButton} onClick={() => api.trigger(GROUP, "jumpTo", h.x, h.y, h.z)}>
                {t.goSee}
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function Households({ report, t }: { report: Report; t: T }) {
  const h = report.households;
  return (
    <div style={s.section}>
      <div style={s.heading}>{t.householdsHeading}</div>
      <Row label={t.homeless} value={fmt(h?.homeless)} />
      <Row label={t.movingAway} value={fmt(h?.movingAway)} />
      <div style={{ ...s.sub, marginTop: "8rem" }}>{t.scanTime(report.scanMs)}</div>
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
  const t = useTexts();
  const statusText = api.useValue(status$);
  const report = React.useMemo(() => parseReport(statusText), [statusText]);
  return (
    <div>
      <Findings report={report} t={t} />
      <Speed report={report} t={t} />
      <Pathfind report={report} t={t} />
      <Jobs report={report} t={t} />
      <Households report={report} t={t} />
    </div>
  );
}
