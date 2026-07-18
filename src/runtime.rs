//! ランタイムアダプタ層(コアとフレームワークの境界)。
//!
//! [`crate::event_loop::EventLoop`] は論理時刻ベースの純粋モデルであり、
//! tokio にも RPoem にも依存しない。実ランタイムに載せる際は、この
//! モジュールの [`LoopDriver`] トレイトを実装したアダプタが
//! 「実時間 → 論理時刻」の変換と待機を担う、という層分離にする。
//!
//! - コア(`resolve`/`event_loop`): 依存クレートゼロ、純粋関数/純粋キュー。
//! - アダプタ(この層): `LoopDriver` 実装ごとに分離。テスト用の
//!   [`ManualDriver`] は本体に同梱、tokio/RPoem 用アダプタは feature
//!   (`rpoem-adapter`)配下の雛形として分離(v0.1.0 では設計スタブのみ)。

use crate::event_loop::{EventLoop, Executed};

/// イベントループを実ランタイム上で駆動するアダプタの抽象。
///
/// 実装者は「次のタイマー満期まで待つ」手段(tokio の `sleep`、RPoem の
/// スケジューラ等)を提供する。コア側は `next_timer_due()` を公開して
/// いるので、アダプタは次の満期まで待って `advance_to` → `run_ready` を
/// 繰り返せばよい。
pub trait LoopDriver {
    /// 論理時刻 `due`(ミリ秒相当)まで「待った」ことにして現在時刻を返す。
    /// 実ランタイム実装ではここで実際にスリープする。
    fn wait_until(&mut self, due: u64) -> u64;
}

/// テスト・シミュレーション用のドライバ。待機は論理時刻を進めるだけ。
#[derive(Debug, Default)]
pub struct ManualDriver;

impl LoopDriver for ManualDriver {
    fn wait_until(&mut self, due: u64) -> u64 {
        due
    }
}

/// ドライバを使ってイベントループをアイドルまで駆動する。
///
/// `EventLoop::run_until_idle` と同じ結果になるが、時間を進める判断を
/// [`LoopDriver`] に委譲する点が異なる(実ランタイム統合の骨格)。
pub fn drive_until_idle(el: &mut EventLoop, driver: &mut dyn LoopDriver) -> Vec<Executed> {
    let mut trace = Vec::new();
    loop {
        trace.extend(el.run_ready());
        match el.next_timer_due() {
            Some(due) => {
                let now = driver.wait_until(due);
                el.advance_to(now);
            }
            None => break,
        }
    }
    trace
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_driver_matches_run_until_idle() {
        let build = || {
            let mut el = EventLoop::new();
            el.schedule_timer(1, 50);
            el.schedule_next_tick(0);
            el.schedule_timer(2, 100);
            el
        };
        let mut a = build();
        let expected = a.run_until_idle();

        let mut b = build();
        let mut driver = ManualDriver;
        let got = drive_until_idle(&mut b, &mut driver);
        assert_eq!(got, expected);
        assert!(b.is_idle());
    }
}
