// mod の UI の入口。ゲームが CityCheckup.mjs を読み込み、default export を呼んで部品を登録させる。
import type { ModuleRegistry } from "./game";
import { Root } from "./root";

export default function register(moduleRegistry: ModuleRegistry): void {
  // 画面左上（情報ビューのボタン列の右）に開閉ボタンとパネルを置く。
  moduleRegistry.append("GameTopLeft", Root);
}
