// パネル全体で共通の定数・書式・見た目。

// C# 側 PanelUISystem.Group と一致させること。
export const GROUP = "cityCheckup";

// ゲームの UI エンジン（Coherent Gameface）は toLocaleString の書式指定を無視するため、桁区切りは自前で付ける。
export function fmt(n: number | null | undefined, digits = 0): string {
  if (n === null || n === undefined || !isFinite(n)) {
    return "—";
  }
  const [int, frac] = n.toFixed(digits).split(".");
  const grouped = int.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return frac ? `${grouped}.${frac}` : grouped;
}

export const pct = (n: number | null | undefined) => (n === null || n === undefined ? "—" : `${Math.round(n * 100)}%`);

export const colors = {
  panel: "rgba(24, 33, 51, 0.92)",
  line: "rgba(255, 255, 255, 0.15)",
  sub: "rgba(255, 255, 255, 0.65)",
  warn: "#ffb74d",
  bar: "#4fc3f7",
  button: "rgba(79, 195, 247, 0.25)",
  tabActive: "rgba(79, 195, 247, 0.55)",
};

export const s = {
  toggle: {
    pointerEvents: "auto",
    margin: "6rem",
    padding: "6rem 12rem",
    background: "rgba(24, 33, 51, 0.85)",
    color: "white",
    borderRadius: "4rem",
    fontSize: "14rem",
  },
  panel: {
    pointerEvents: "auto",
    margin: "0 6rem",
    padding: "10rem 14rem",
    width: "420rem",
    // 画面の高さに合わせ、下のツールバーにかからない程度まで伸ばす。
    maxHeight: "78vh",
    overflowY: "scroll",
    background: colors.panel,
    color: "white",
    borderRadius: "4rem",
    fontSize: "13rem",
  },
  section: { marginTop: "10rem", paddingTop: "8rem", borderTop: `1rem solid ${colors.line}` },
  heading: { fontSize: "14rem", fontWeight: "bold", marginBottom: "4rem" },
  row: { display: "flex", flexDirection: "row", justifyContent: "space-between", alignItems: "center", marginTop: "2rem" },
  sub: { color: colors.sub },
  smallButton: {
    pointerEvents: "auto",
    padding: "2rem 8rem",
    background: colors.button,
    color: "white",
    borderRadius: "3rem",
    fontSize: "12rem",
  },
};
