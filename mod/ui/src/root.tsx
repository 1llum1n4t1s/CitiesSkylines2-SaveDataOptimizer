// 開閉ボタンと、タブで切り替えるパネル本体（「負荷分析」「欠落アセット」）。
import { React, api } from "./game";
import { MissingTab } from "./missing";
import { LoadTab, loadReady } from "./panel";
import { GROUP, colors, s } from "./style";
import { VehiclesTab } from "./vehicles";

// 動作確認用（Mods\SaveDataOptimizer\debug-* があるとき）。パネルを開いた状態・指定のタブで始める。
const debugOpen$ = api.bindValue<boolean>(GROUP, "debugOpen", false);
const debugTab$ = api.bindValue<string>(GROUP, "debugTab", "load");
// ゲームで何かを選択中か（選択中はゲームの情報パネルと重なるので、こちらのパネルを畳む）。
const hasSelection$ = api.bindValue<boolean>(GROUP, "hasSelection", false);

type Tab = "load" | "missing" | "vehicles";

const TABS: { key: Tab; label: string }[] = [
  { key: "load", label: "負荷分析" },
  { key: "missing", label: "欠落アセット" },
  { key: "vehicles", label: "取り残し車両" },
];

export function Root() {
  const debugOpen = api.useValue(debugOpen$);
  const debugTab = api.useValue(debugTab$);
  const ready = loadReady();
  const hasSelection = api.useValue(hasSelection$);
  const [open, setOpen] = React.useState(false);
  const [tab, setTab] = React.useState<Tab>("load");
  const [reported, setReported] = React.useState(false);

  React.useEffect(() => {
    if (debugOpen) {
      setOpen(true);
      setTab(debugTab === "missing" || debugTab === "vehicles" ? debugTab : "load");
    }
  }, [debugOpen, debugTab]);

  // 初めて C# の値が届いたら、表示できたことを C# 側へ知らせる（Player.log で確認するため）。
  React.useEffect(() => {
    if (!reported && ready) {
      api.trigger(GROUP, "uiMounted", "root");
      setReported(true);
    }
  }, [reported, ready]);

  // 負荷の集計は「負荷分析」タブを開いている間だけ動かす（C# 側でも重い処理を止める）。
  React.useEffect(() => {
    api.trigger(GROUP, "setOpen", open && tab === "load");
  }, [open, tab]);

  return (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "flex-start" }}>
      <button
        style={s.toggle}
        onClick={() => {
          if (open && hasSelection) {
            // 畳まれているときに押したら、選択を解いてパネルを見せ直す。
            api.trigger(GROUP, "clearSelection");
          } else {
            setOpen(!open);
          }
        }}
      >
        {open && hasSelection ? "街の診断（選択中は畳んでいます）" : "街の診断"}
      </button>
      {open && !hasSelection && (
        <div style={s.panel}>
          <div style={{ display: "flex", flexDirection: "row", marginBottom: "8rem" }}>
            {TABS.map((t) => (
              <button
                key={t.key}
                style={{ ...s.smallButton, marginRight: "6rem", fontSize: "13rem", background: t.key === tab ? colors.tabActive : colors.button }}
                onClick={() => setTab(t.key)}
              >
                {t.label}
              </button>
            ))}
          </div>
          {tab === "load" ? <LoadTab /> : tab === "missing" ? <MissingTab /> : <VehiclesTab />}
        </div>
      )}
    </div>
  );
}
