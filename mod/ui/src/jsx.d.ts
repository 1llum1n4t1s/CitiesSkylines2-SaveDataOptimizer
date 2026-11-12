// JSX の最小限の型。React 本体はゲームが window に公開するため @types/react を入れず、
// 要素名と属性は緩く受け付ける（属性の綴り間違いは実機確認で見つける前提）。
/* eslint-disable @typescript-eslint/no-explicit-any */
declare namespace JSX {
  type Element = any;
  interface IntrinsicElements {
    [name: string]: any;
  }
}
