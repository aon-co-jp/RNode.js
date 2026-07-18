# RNode.js

Node.js のコア概念を、既存の Node.js 実装コードを一切流用せず Rust で
一から再実装するプロジェクト([RFrontEnd](https://github.com/aon-co-jp)
エコシステム傘下)。

## これは何か(v0.1.0 の正直なスコープ)

Node.js は巨大であり、JavaScript エンジン(V8 相当)を一から作るのは
非現実的なため、v0.1.0 は **「Node.js 全体」ではなく次の 2 つの概念モデルの
骨格だけ**を実装している:

- **モジュール解決**(`resolve`): CommonJS 的な `require(path)` の
  パス解決アルゴリズム——相対パス・拡張子補完(`.js` → `.json`)・
  ディレクトリの `index`・`package.json` の `main`・`node_modules` の
  祖先方向探索——を、実ファイルシステムアクセスから切り離した純粋関数
  として実装。存在判定は `FileSystem` トレイトで抽象化され、テストは
  インメモリの `MapFileSystem` で実 I/O 無しに解決規則そのものを検証する。
- **イベントループ順序モデル**(`event_loop`): next-tick キュー →
  マイクロタスクキュー → タイマーフェーズという実行順序規則を、実時間や
  tokio に載せる前の**論理時刻ベースの決定的な純粋キューロジック**として
  実装。`setTimeout(0)` でも Promise `then` より後、タイマー 1 件ごとに
  tick/microtask を完全排出、といった Node の代表的挙動をテストで固定。
- **ランタイムアダプタ層**(`runtime`): 純粋モデルと実ランタイム
  (tokio/RPoem)の境界となる `LoopDriver` トレイト。テスト用
  `ManualDriver` 同梱。RPoem 統合は feature `rpoem-adapter` の設計
  スタブのみ(実装は未着手)。

**未実装(スコープ外)**: JavaScript の実行そのもの、コアモジュール
(`fs`/`http` 等)、`setImmediate`/I/O ポーリング/close フェーズ、
`package.json` の `exports`、ESM。RTypeScript との将来連携構想は
`CLAUDE.md` の設計メモを参照。

## 使用例

```rust
use rnodejs::{resolve, MapFileSystem, EventLoop};

// モジュール解決(実 I/O 無し)
let fs = MapFileSystem::new()
    .with_file("/app/node_modules/lodash/index.js")
    .with_file("/app/src/util.js");
assert_eq!(resolve("lodash", "/app/src", &fs).unwrap(),
           "/app/node_modules/lodash/index.js");
assert_eq!(resolve("./util", "/app/src", &fs).unwrap(),
           "/app/src/util.js");

// イベントループ順序モデル(論理時刻・決定的)
let mut el = EventLoop::new();
el.schedule_timer(3, 0);      // setTimeout(0) 相当
el.schedule_microtask(2);     // Promise.then 相当
el.schedule_next_tick(1);     // process.nextTick 相当
let trace = el.run_until_idle();
assert_eq!(trace.iter().map(|e| e.id).collect::<Vec<_>>(), vec![1, 2, 3]);
```

## ビルド・テスト

```bash
cargo test
cargo test --features rpoem-adapter
```

依存クレートゼロ(`unsafe_code = "deny"`)。追加のセットアップは不要。

## 関連プロジェクト

- [RPoem](https://github.com/aon-co-jp/RPoem) — tokio/hyper ベースの
  サーバー側実行基盤。I/O・HTTP は RPoem が担い、RNode.js は
  「Node.js 的な JavaScript ランタイム/モジュールシステムの土台」に
  焦点を当てる(役割重複を避ける分担)。
- [RTypeScript](https://github.com/aon-co-jp/RTypeScript) — トークナイザ+
  型注釈除去トランスパイラ。将来の JS 実行層の連携候補(`CLAUDE.md` 参照)。
- [RJSON](https://github.com/aon-co-jp/RJSON) — 本リポジトリの構成・
  ドキュメント体裁の雛形。

## ライセンス

Apache-2.0 OR MIT
