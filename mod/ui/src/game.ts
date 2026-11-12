// ゲーム本体の UI が window に公開しているモジュールを受け取る窓口。
//
// ゲームの index.js は React と "cs2/api" などを window のプロパティとして公開し、
// mod の .mjs を import() で読み込んだあと default export の関数へ登録口（moduleRegistry）を渡す。
// import 文で "cs2/api" を書くとブラウザーが解決できないため、ここで window から取り出して型を付ける。
// 型はこの mod で使う範囲だけを手書きしている（公式の型定義パッケージには依存しない）。

/* eslint-disable @typescript-eslint/no-explicit-any */
const w = window as any;

export interface ReactLike {
  createElement: (...args: any[]) => any;
  Fragment: any;
  useState<T>(initial: T): [T, (value: T | ((prev: T) => T)) => void];
  useEffect(effect: () => void | (() => void), deps?: unknown[]): void;
  useMemo<T>(factory: () => T, deps: unknown[]): T;
}

/** ゲームのバインディング値。useValue で React から購読する。 */
export interface ValueBinding<T> {
  readonly value: T;
}

export interface Cs2Api {
  bindValue<T>(group: string, name: string, fallback?: T): ValueBinding<T>;
  useValue<T>(binding: ValueBinding<T>): T;
  trigger(group: string, name: string, ...args: unknown[]): void;
}

/** mod の default export が受け取る登録口。append で決められた場所へ部品を差し込む。 */
export interface ModuleRegistry {
  append(location: string, component: any): void;
}

export const React: ReactLike = w.React;
export const api: Cs2Api = w["cs2/api"];
