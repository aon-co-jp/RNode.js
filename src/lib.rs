//! # RNode.js
//!
//! Node.js のコア概念を、既存の Node.js 実装コードを一切流用せず
//! Rust で一から再実装するプロジェクト(RFrontEnd エコシステム傘下)。
//!
//! v0.1.0 のスコープは「Node.js 全体」ではなく、次の 2 つの概念モデルの
//! 骨格に意図的に絞っている:
//!
//! - [`resolve`]: CommonJS 的な `require(path)` のパス解決アルゴリズム
//!   (相対パス・拡張子補完 `.js`/`.json`・ディレクトリ `index`・
//!   `package.json` の `main`・`node_modules` 探索)を、実ファイルシステム
//!   アクセス抜きで単体テストできる純粋関数として実装。
//!
//! JavaScript の実行そのもの(V8 相当)は本クレートのスコープ外。
//! 将来的な RTypeScript との連携構想は `CLAUDE.md` の設計メモを参照。

#![deny(unsafe_code)]

pub mod resolve;

pub use resolve::{resolve, FileSystem, MapFileSystem, ResolveError};
