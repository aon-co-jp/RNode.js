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

/// tokio ホストアプリケーション(RPoem等)向けの実時間駆動
/// [`LoopDriver`](feature `tokio-driver`)。
///
/// [`ManualDriver`] が論理時刻を即座に進めるだけなのに対し、こちらは
/// 生成時刻を論理時刻 0 の基準点として、論理時刻 `due`(ミリ秒相当)を
/// 「基準点からの経過実時間」にマッピングし、その残り時間だけ実際に待つ。
/// これにより `event_loop.rs` の決定的キューロジック(順序規則)と実時間・
/// 実タイマーを接続する。
///
/// **実装メモ(2026-07-18、実バグ修正)**: [`LoopDriver::wait_until`] は
/// 同期メソッドであり、呼び出し側スレッドを実際にブロックする設計。
/// 当初`tokio::runtime::Handle::current().block_on(tokio::time::sleep(..))`
/// で実装していたが、`Runtime::enter()`のガードのみでランタイム自体を
/// 駆動していない(current_threadフレーバーの場合、タイマー/IOドライバは
/// `Runtime::block_on`が持つpark/wakeループでのみ駆動される)状況で
/// `Handle::block_on`を呼ぶと、タイマーが永久に満期を迎えずハングする
/// 実バグを`cargo test`で発見した(2026-07-18)。`wait_until`は元々
/// 同期関数で呼び出しスレッドを止める設計なので、非同期ランタイムを
/// 介する必要はそもそも無く、`std::thread::sleep`に置き換えて解消した
/// (tokioの実行スレッドをこの中で同期的にブロックするのは、そもそも
/// tokioワーカースレッド上では避けるべきアンチパターンでもある)。
#[cfg(feature = "tokio-driver")]
#[derive(Debug)]
pub struct TokioDriver {
    /// 論理時刻 0 に対応する実時刻の基準点。
    start: std::time::Instant,
}

#[cfg(feature = "tokio-driver")]
impl TokioDriver {
    /// 現在時刻を論理時刻 0 の基準点として新しいドライバを作る。
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }
}

#[cfg(feature = "tokio-driver")]
impl Default for TokioDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "tokio-driver")]
impl LoopDriver for TokioDriver {
    fn wait_until(&mut self, due: u64) -> u64 {
        let target = self.start + std::time::Duration::from_millis(due);
        let now = std::time::Instant::now();
        if target > now {
            std::thread::sleep(target - now);
        }
        due
    }
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

    #[cfg(feature = "tokio-driver")]
    #[test]
    fn tokio_driver_preserves_deterministic_order() {
        // wait_until は std::thread::sleep ベースの同期実装のため、tokio
        // ランタイムを起動・enter() する必要はない(2026-07-18の修正で
        // Handle::block_on 依存を撤去したため)。
        let mut el = EventLoop::new();
        el.schedule_timer(1, 20);
        el.schedule_next_tick(0);
        el.schedule_timer(2, 40);

        let mut driver = TokioDriver::new();
        let trace = drive_until_idle(&mut el, &mut driver);

        // 順序規則は ManualDriver/純粋モデルと同じ(next-tick が先、
        // タイマーは満期の早い順)。
        assert_eq!(
            trace.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(el.is_idle());
    }

    #[cfg(feature = "tokio-driver")]
    #[test]
    fn tokio_driver_actually_waits_real_time() {
        let mut el = EventLoop::new();
        el.schedule_timer(1, 30);

        let mut driver = TokioDriver::new();
        let started = std::time::Instant::now();
        let trace = drive_until_idle(&mut el, &mut driver);
        let elapsed = started.elapsed();

        assert_eq!(trace.iter().map(|e| e.id).collect::<Vec<_>>(), vec![1]);
        assert!(
            elapsed >= std::time::Duration::from_millis(25),
            "実時間で少なくともタイマーの満期近くまで待つはず: {elapsed:?}"
        );
    }
}
