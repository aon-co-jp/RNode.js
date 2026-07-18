//! Node.js イベントループの順序モデルの概念実装。
//!
//! 実際の tokio ランタイムや OS タイマーに載せる前段階として、
//! **「どのキューのタスクが、どの順序で実行されるか」という順序規則だけ**を
//! 純粋なデータ構造として実装し、単体テストで検証する。実時間は使わず、
//! 論理時刻(`u64` のミリ秒相当)を外から与える決定的モデルなので、
//! テストは常に再現可能。
//!
//! 写し取った Node.js の概念(実装コードは一切流用していない):
//!
//! - **next-tick キュー**(`process.nextTick`): マイクロタスクより先に、
//!   各タスクの実行直後に必ず空になるまで排出される最優先キュー。
//! - **マイクロタスクキュー**(Promise の `then` 相当): next-tick キューが
//!   空になった後に排出される。
//! - **タイマーフェーズ**(`setTimeout`): 論理時刻が満期に達したタイマーを
//!   登録順(同時刻なら先に登録した方が先)に実行する。
//! - 各マクロタスク(タイマー)を 1 つ実行するたびに、next-tick →
//!   マイクロタスクの順で両キューを完全に排出してから次のマクロタスクへ
//!   進む(Node の「マクロタスク 1 件ごとにマイクロタスク排出」の規則)。
//!
//! v0.1.0 で**モデル化していない**もの: `setImmediate`/check フェーズ、
//! I/O ポーリングフェーズ、close コールバック、実スレッド・実タイマー。

use std::collections::VecDeque;

/// スケジュールされたタスクの識別子。実行順の検証(テスト)ではこの ID の
/// 並びを観測する。実際のクロージャ実行はこのモデルの責務外
/// (tokio 統合の段階で導入する)。
pub type TaskId = u64;

/// タスクの種別。`run_until_idle` が返す実行トレースに現れる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    /// `process.nextTick` 相当。
    NextTick,
    /// Promise `then` 相当のマイクロタスク。
    Microtask,
    /// `setTimeout` 相当のタイマー。
    Timer,
}

/// 実行トレースの 1 エントリ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Executed {
    pub kind: TaskKind,
    pub id: TaskId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TimerEntry {
    /// 満期の論理時刻(ミリ秒相当)。
    due: u64,
    /// 同時刻タイマーの登録順タイブレーク。
    seq: u64,
    id: TaskId,
}

/// イベントループの順序モデル本体。
///
/// 論理時刻は [`EventLoop::advance_to`] で外部から進める(実時間は見ない)。
/// タスクの「実行」は ID をトレースに記録することを意味する。
#[derive(Debug, Default)]
pub struct EventLoop {
    next_tick_queue: VecDeque<TaskId>,
    microtask_queue: VecDeque<TaskId>,
    timers: Vec<TimerEntry>,
    /// 現在の論理時刻。
    now: u64,
    /// タイマー登録順のタイブレーク用連番。
    timer_seq: u64,
}

impl EventLoop {
    pub fn new() -> Self {
        Self::default()
    }

    /// 現在の論理時刻。
    pub fn now(&self) -> u64 {
        self.now
    }

    /// `process.nextTick(id)` 相当。
    pub fn schedule_next_tick(&mut self, id: TaskId) {
        self.next_tick_queue.push_back(id);
    }

    /// Promise `then` 相当のマイクロタスクを積む。
    pub fn schedule_microtask(&mut self, id: TaskId) {
        self.microtask_queue.push_back(id);
    }

    /// `setTimeout(id, delay_ms)` 相当。現在の論理時刻 + `delay_ms` で満期。
    pub fn schedule_timer(&mut self, id: TaskId, delay_ms: u64) {
        let entry = TimerEntry {
            due: self.now.saturating_add(delay_ms),
            seq: self.timer_seq,
            id,
        };
        self.timer_seq += 1;
        self.timers.push(entry);
    }

    /// 論理時刻を `to` まで進める(戻すことはできない)。
    pub fn advance_to(&mut self, to: u64) {
        if to > self.now {
            self.now = to;
        }
    }

    /// 次に満期を迎えるタイマーの満期時刻(未満期のものを含む最小値)。
    /// タイマーが無ければ `None`。「次にいつまで時間を進めれば仕事があるか」を
    /// 呼び出し側(将来の tokio 統合層)が知るためのフック。
    pub fn next_timer_due(&self) -> Option<u64> {
        self.timers.iter().map(|t| t.due).min()
    }

    /// 未処理のタスクが 1 つも無ければ true(タイマーは未満期でも「有る」扱い)。
    pub fn is_idle(&self) -> bool {
        self.next_tick_queue.is_empty() && self.microtask_queue.is_empty() && self.timers.is_empty()
    }

    /// next-tick キュー → マイクロタスクキューの順で両方を完全に排出する。
    ///
    /// Node の規則に倣い、マイクロタスク実行中に積まれた next-tick は
    /// マイクロタスクより先に割り込む……という相互排出を、両キューが
    /// 空になるまで繰り返す。
    fn drain_ticks_and_microtasks(&mut self, trace: &mut Vec<Executed>) {
        loop {
            if let Some(id) = self.next_tick_queue.pop_front() {
                trace.push(Executed { kind: TaskKind::NextTick, id });
                continue;
            }
            if let Some(id) = self.microtask_queue.pop_front() {
                trace.push(Executed { kind: TaskKind::Microtask, id });
                continue;
            }
            break;
        }
    }

    /// 現在の論理時刻で実行可能なタスクをすべて実行し、実行トレースを返す。
    ///
    /// 順序規則:
    /// 1. まず next-tick → マイクロタスクを完全排出。
    /// 2. 満期に達したタイマーを (満期時刻, 登録順) の昇順で 1 つずつ実行し、
    ///    タイマー 1 つ実行するごとに手順 1 の排出を挟む。
    /// 3. 実行可能なものが無くなったら終了(未満期タイマーは残る)。
    pub fn run_ready(&mut self) -> Vec<Executed> {
        let mut trace = Vec::new();
        self.drain_ticks_and_microtasks(&mut trace);
        loop {
            // 満期タイマーのうち (due, seq) 最小のものを選ぶ。
            let pos = self
                .timers
                .iter()
                .enumerate()
                .filter(|(_, t)| t.due <= self.now)
                .min_by_key(|(_, t)| (t.due, t.seq))
                .map(|(i, _)| i);
            match pos {
                Some(i) => {
                    let entry = self.timers.remove(i);
                    trace.push(Executed { kind: TaskKind::Timer, id: entry.id });
                    self.drain_ticks_and_microtasks(&mut trace);
                }
                None => break,
            }
        }
        trace
    }

    /// すべてのタスクが尽きるまで、必要に応じて論理時刻を次のタイマー満期へ
    /// 自動で進めながら実行する。決定的なシミュレーション実行。
    pub fn run_until_idle(&mut self) -> Vec<Executed> {
        let mut trace = Vec::new();
        loop {
            trace.extend(self.run_ready());
            match self.next_timer_due() {
                Some(due) => self.advance_to(due),
                None => break,
            }
        }
        trace
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(trace: &[Executed]) -> Vec<TaskId> {
        trace.iter().map(|e| e.id).collect()
    }

    #[test]
    fn next_tick_runs_before_microtask() {
        let mut el = EventLoop::new();
        el.schedule_microtask(2);
        el.schedule_next_tick(1);
        let trace = el.run_ready();
        assert_eq!(ids(&trace), vec![1, 2], "nextTick が Promise then より先");
    }

    #[test]
    fn microtasks_run_before_timers() {
        let mut el = EventLoop::new();
        el.schedule_timer(3, 0);
        el.schedule_microtask(2);
        el.schedule_next_tick(1);
        let trace = el.run_ready();
        assert_eq!(ids(&trace), vec![1, 2, 3]);
    }

    #[test]
    fn timers_fire_in_due_order() {
        let mut el = EventLoop::new();
        el.schedule_timer(20, 20);
        el.schedule_timer(10, 10);
        let trace = el.run_until_idle();
        assert_eq!(ids(&trace), vec![10, 20]);
    }

    #[test]
    fn same_due_timers_fire_in_registration_order() {
        let mut el = EventLoop::new();
        el.schedule_timer(1, 5);
        el.schedule_timer(2, 5);
        el.schedule_timer(3, 5);
        let trace = el.run_until_idle();
        assert_eq!(ids(&trace), vec![1, 2, 3]);
    }

    #[test]
    fn timer_does_not_fire_before_due() {
        let mut el = EventLoop::new();
        el.schedule_timer(1, 100);
        assert!(el.run_ready().is_empty(), "満期前のタイマーは実行されない");
        el.advance_to(99);
        assert!(el.run_ready().is_empty());
        el.advance_to(100);
        assert_eq!(ids(&el.run_ready()), vec![1]);
        assert!(el.is_idle());
    }

    #[test]
    fn ticks_scheduled_during_timer_phase_interleave() {
        // タイマー2つの間に nextTick/マイクロタスクを挟むと、
        // タイマー1つごとに排出される(まとめて最後ではない)ことを、
        // 「タイマー実行後に積む」状況を模して検証する。
        let mut el = EventLoop::new();
        el.schedule_timer(1, 0);
        el.schedule_timer(4, 10);
        // 論理時刻0でタイマー1を実行。
        let t1 = el.run_ready();
        assert_eq!(ids(&t1), vec![1]);
        // タイマー1のコールバック内で積まれた想定のタスク。
        el.schedule_next_tick(2);
        el.schedule_microtask(3);
        let rest = el.run_until_idle();
        // tick/microtask はタイマー4より先に排出される。
        assert_eq!(ids(&rest), vec![2, 3, 4]);
        assert_eq!(
            rest.iter().map(|e| e.kind).collect::<Vec<_>>(),
            vec![TaskKind::NextTick, TaskKind::Microtask, TaskKind::Timer]
        );
    }

    #[test]
    fn next_tick_scheduled_by_microtask_preempts_remaining_microtasks() {
        // Node の規則: マイクロタスク処理中でも nextTick キューが優先。
        // モデル上は「microtask 3 の実行後に nextTick 9 が積まれた」状況を
        // 排出ループの途中割り込みとして検証する。
        let mut el = EventLoop::new();
        el.schedule_microtask(3);
        let t = el.run_ready();
        assert_eq!(ids(&t), vec![3]);
        el.schedule_next_tick(9);
        el.schedule_microtask(4);
        let t2 = el.run_ready();
        assert_eq!(ids(&t2), vec![9, 4], "後から積んだ nextTick が microtask より先");
    }

    #[test]
    fn run_until_idle_auto_advances_time() {
        let mut el = EventLoop::new();
        el.schedule_timer(1, 50);
        el.schedule_timer(2, 200);
        let trace = el.run_until_idle();
        assert_eq!(ids(&trace), vec![1, 2]);
        assert_eq!(el.now(), 200, "論理時刻は最後のタイマー満期まで進む");
        assert!(el.is_idle());
    }

    #[test]
    fn next_timer_due_reports_earliest() {
        let mut el = EventLoop::new();
        assert_eq!(el.next_timer_due(), None);
        el.schedule_timer(1, 30);
        el.schedule_timer(2, 10);
        assert_eq!(el.next_timer_due(), Some(10));
    }

    #[test]
    fn zero_delay_timer_still_after_microtasks() {
        // setTimeout(0) でも Promise then より後、という Node の代表的挙動。
        let mut el = EventLoop::new();
        el.schedule_timer(2, 0);
        el.schedule_microtask(1);
        let trace = el.run_until_idle();
        assert_eq!(ids(&trace), vec![1, 2]);
    }
}
