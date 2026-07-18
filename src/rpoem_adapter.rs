//! RPoem 統合アダプタ(feature `rpoem-adapter`)。
//!
//! RPoem(tokio/hyper ベースのサーバー側実行基盤)本体への実依存は
//! まだ追加しない——コアを依存クレートゼロに保つため、この feature
//! 配下だけに依存を閉じ込める方針は変えない。ここで定義するのは
//! 「RPoem 側の実装がまだ無くても成立する、最小限の具体的な
//! インターフェース」であり、実際の RPoem リクエストハンドラとの
//! 結線は将来の作業。
//!
//! 役割分担: RPoem が I/O・HTTP(実際のソケット・タイマー駆動含む)を
//! 担い、RNode.js は「Node.js 的なタスク順序モデル」
//! ([`crate::event_loop::EventLoop`])を担う。この境界を
//! [`TaskSubmitter`] トレイトと [`RPoemAdapter`] 構造体として固定する。
//!
//! 駆動そのもの([`crate::runtime::LoopDriver`] 実装)は RPoem に
//! 依存しない——tokio ランタイム上で走らせる場合は feature
//! `tokio-driver` の [`crate::runtime::TokioDriver`] をそのまま
//! [`RPoemAdapter`] に差し込める(この 2 つの feature は独立しており、
//! 併用も単独使用もできる)。

use crate::event_loop::{EventLoop, Executed, TaskId};
use crate::runtime::{drive_until_idle, LoopDriver};

/// RPoem のリクエストハンドラ側から `EventLoop` へタスクを投入するための
/// ブリッジ抽象。RPoem 側の実装はこのトレイトを実装したアダプタ経由で
/// `process.nextTick` / Promise `then` / `setTimeout` 相当のタスクを
/// 積むことを想定する。
pub trait TaskSubmitter {
    /// `process.nextTick(id)` 相当のタスクを投入する。
    fn submit_next_tick(&mut self, id: TaskId);

    /// Promise `then` 相当のマイクロタスクを投入する。
    fn submit_microtask(&mut self, id: TaskId);

    /// `setTimeout(id, delay_ms)` 相当のタイマーを投入する。
    fn submit_timer(&mut self, id: TaskId, delay_ms: u64);
}

/// [`EventLoop`](純粋なタスク順序モデル)と [`LoopDriver`] 実装
/// (実行基盤との橋渡し)を束ねる、RPoem 統合の最小アダプタ。
///
/// `D` には [`crate::runtime::ManualDriver`](テスト用)や feature
/// `tokio-driver` の `TokioDriver`(実時間駆動)を差し込める。RPoem
/// 本体固有のドライバを実装したい場合も、`LoopDriver` を実装するだけで
/// このアダプタにそのまま載せられる——これが「RPoem 側の実装が無くても
/// インターフェースとして成立する」ことの意味。
pub struct RPoemAdapter<D: LoopDriver> {
    event_loop: EventLoop,
    driver: D,
}

impl<D: LoopDriver> RPoemAdapter<D> {
    /// 空の `EventLoop` と与えられたドライバでアダプタを作る。
    pub fn new(driver: D) -> Self {
        Self {
            event_loop: EventLoop::new(),
            driver,
        }
    }

    /// 内部の `EventLoop` への参照(タイマー満期の問い合わせなど、
    /// タスク投入以外の用途向け)。
    pub fn event_loop(&self) -> &EventLoop {
        &self.event_loop
    }

    /// アイドルになるまでドライバで駆動し、実行トレースを返す
    /// (`crate::runtime::drive_until_idle` への薄いラッパー)。
    pub fn drive_until_idle(&mut self) -> Vec<Executed> {
        drive_until_idle(&mut self.event_loop, &mut self.driver)
    }
}

impl<D: LoopDriver> TaskSubmitter for RPoemAdapter<D> {
    fn submit_next_tick(&mut self, id: TaskId) {
        self.event_loop.schedule_next_tick(id);
    }

    fn submit_microtask(&mut self, id: TaskId) {
        self.event_loop.schedule_microtask(id);
    }

    fn submit_timer(&mut self, id: TaskId, delay_ms: u64) {
        self.event_loop.schedule_timer(id, delay_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ManualDriver;

    #[test]
    fn rpoem_adapter_preserves_ordering_via_task_submitter() {
        let mut adapter = RPoemAdapter::new(ManualDriver);
        adapter.submit_timer(3, 0);
        adapter.submit_microtask(2);
        adapter.submit_next_tick(1);

        let trace = adapter.drive_until_idle();
        assert_eq!(trace.iter().map(|e| e.id).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert!(adapter.event_loop().is_idle());
    }

    #[test]
    fn rpoem_adapter_reports_next_timer_due_through_event_loop() {
        let mut adapter = RPoemAdapter::new(ManualDriver);
        assert_eq!(adapter.event_loop().next_timer_due(), None);
        adapter.submit_timer(1, 50);
        assert_eq!(adapter.event_loop().next_timer_due(), Some(50));
    }
}
