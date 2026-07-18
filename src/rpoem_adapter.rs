//! RPoem/tokio 統合アダプタの雛形(feature `rpoem-adapter`)。
//!
//! **v0.1.0 では設計スタブのみ**。RPoem(tokio/hyper ベースのサーバー側
//! 実行基盤)への実依存はまだ追加しない——コアを依存クレートゼロに保つ
//! ため、実装時もこの feature 配下だけに依存を閉じ込める方針。
//!
//! 実装予定(未実装):
//! - `TokioDriver`: [`crate::runtime::LoopDriver`] の tokio 実装。
//!   `wait_until(due)` を `tokio::time::sleep_until` で待つ。
//! - RPoem のリクエストハンドラから `EventLoop` へタスクを投入する
//!   ブリッジ(RPoem 側が I/O・HTTP を担い、RNode.js 側はモジュール
//!   解決とタスク順序を担う、という役割分担)。
//!
//! ここに実コードを書く前に、コア側 API(`next_timer_due` /
//! `advance_to` / `run_ready`)だけで駆動できることを
//! `runtime::ManualDriver` のテストで担保してある。

// (意図的に空。設計メモは上記ドキュメントコメントと CLAUDE.md を参照)
