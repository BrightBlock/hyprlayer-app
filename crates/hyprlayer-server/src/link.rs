//! How much a `LINK_CREDIT` peer's link may hold unread (ADR-0009): `GAIN` times the
//! link's recent peak rate times its smallest round trip, never under `LINK_FLOOR`. The
//! peer's `received` reports, and the times it stamps on them, are the only signal.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// At 64 KiB/s a small call waits 2 s behind this much.
const LINK_FLOOR: u64 = 128 * 1024;
/// The most one wrong estimate can put on the link.
const LINK_MAX: u64 = 8 * 1024 * 1024;
/// The bandwidth-delay product plus half again queued: after a rate drop a small call
/// waits about `GAIN` × min RTT × old rate ÷ new rate.
const GAIN: f64 = 1.5;
/// Without a rate, startup grows eightfold a round trip; 8 × one part is `LINK_FLOOR`, so
/// a link that one part already fills gets no more.
const STARTUP_GAIN: f64 = 8.0;
/// A frame this small on an empty link times the bare round trip.
const BASELINE_BYTES: u64 = 4 * 1024;
/// Below this a spread is noise, not the link's rate.
const MIN_SPREAD: Duration = Duration::from_millis(1);
/// Writes this close together share one sample and one burst.
const SAMPLE_GAP: Duration = Duration::from_millis(1);
const SAMPLE_BYTES: u64 = 16 * 1024;
const MIN_RATE_SPAN: Duration = Duration::from_millis(10);
/// The peak rate is taken over this many smallest round trips, and at least `MIN_PEAK_SPAN`.
const PEAK_ROUNDS: u32 = 4;
const MIN_PEAK_SPAN: Duration = Duration::from_millis(200);
/// Drained this long, or never once filling the window, the link may have changed.
const IDLE: Duration = Duration::from_secs(1);
/// No round trip near the smallest for this long: the path may have grown, so measure it.
const PROBE_AFTER: Duration = Duration::from_secs(2);
/// What a probe lets the link hold, so its sample shows the path and not a queue.
const PROBE_BYTES: u64 = 16 * 1024;
/// How far answers and events may run past the window: enough that they don't wait on
/// reports held up behind an upload, little enough that a flood of them can't fill a link.
const SPARE_BYTES: u64 = LINK_FLOOR;

/// Bytes written within one sample.
struct Sent {
    start: u64,
    end: u64,
    at: Instant,
    /// In flight once its last byte was written, itself included.
    ahead: u64,
    /// Half the window was used and more was waiting, so this sample measures the link, not an idle sender.
    full: bool,
    /// Written before any report since the link was empty, so `ahead` is all queued behind it.
    first_round: bool,
    burst: u64,
}

pub(crate) struct LinkWindow {
    window: u64,
    written: u64,
    acked: u64,
    in_flight: VecDeque<Sent>,
    /// `(when the peer read it, acked)` over the last rate span; the front is the base.
    acks: VecDeque<(f64, u64)>,
    /// `(when, rate)` over the last peak span.
    rates: VecDeque<(Instant, f64)>,
    /// Smallest since the path was last measured: a larger one can only include a queue.
    min_rtt: Option<Duration>,
    /// Smallest for a frame sent alone, so a burst's lateness past it is the link's rate.
    baseline: Option<Duration>,
    /// A second small frame agreed with it, so one slow one can't pass for the path.
    baseline_confirmed: bool,
    /// When a round trip near the smallest was last seen.
    near_min_at: Option<Instant>,
    /// Since when the window has been held to `PROBE_BYTES` to measure the path.
    probe: Option<Instant>,
    /// Smallest gap between a report's arrival and the peer's stamp on it; more is the
    /// report held up on its way back, not this link.
    min_lag: Option<f64>,
    /// Writes less than `SAMPLE_GAP` apart are one burst; its reports' spread is the link's rate.
    burst: u64,
    last_write: Option<Instant>,
    /// The burst being reported: its id, what was acked by its first report, when that was read.
    train: Option<(u64, u64, f64)>,
    /// Until a full window first shows a queue; again once the link drains.
    startup: bool,
    /// A report arrived since the link was last empty.
    reported: bool,
    last_ack: Option<Instant>,
    last_full: Instant,
    epoch: Instant,
}

impl LinkWindow {
    pub(crate) fn new() -> Self {
        let now = Instant::now();
        Self {
            window: LINK_FLOOR,
            written: 0,
            acked: 0,
            in_flight: VecDeque::new(),
            acks: VecDeque::new(),
            rates: VecDeque::new(),
            min_rtt: None,
            baseline: None,
            baseline_confirmed: false,
            near_min_at: None,
            probe: None,
            min_lag: None,
            burst: 0,
            last_write: None,
            train: None,
            startup: true,
            reported: false,
            last_ack: None,
            last_full: now,
            epoch: now,
        }
    }

    pub(crate) fn written(&self) -> u64 {
        self.written
    }

    pub(crate) fn acked(&self) -> u64 {
        self.acked
    }

    pub(crate) fn has_room(&self) -> bool {
        self.written - self.acked < self.limit()
    }

    pub(crate) fn has_spare(&self) -> bool {
        self.written - self.acked < self.limit() + SPARE_BYTES
    }

    fn limit(&self) -> u64 {
        match self.probe {
            Some(_) => PROBE_BYTES.min(self.window),
            None => self.window,
        }
    }

    /// `waiting`: the writer has more queued behind this.
    pub(crate) fn wrote(&mut self, bytes: usize, waiting: bool, now: Instant) {
        if self.written == self.acked
            && self
                .last_ack
                .is_some_and(|at| now.duration_since(at) >= IDLE)
        {
            // Idle long enough for the path to have moved: measure it afresh.
            self.forget(now);
            self.min_rtt = None;
            self.baseline = None;
            self.baseline_confirmed = false;
            self.min_lag = None;
            self.near_min_at = None;
            self.probe = None;
        }
        if self.acks.is_empty() {
            self.acks.push_back((self.peer_clock(now), self.acked));
        }
        if self
            .last_write
            .is_none_or(|at| now.duration_since(at) >= SAMPLE_GAP)
        {
            self.burst += 1;
        }
        self.last_write = Some(now);
        self.written += bytes as u64;
        let ahead = self.written - self.acked;
        let full = waiting && 2 * ahead >= self.window;
        let first_round = !self.reported;
        match self.in_flight.back_mut() {
            Some(last)
                if now.duration_since(last.at) < SAMPLE_GAP
                    && self.written - last.start <= SAMPLE_BYTES =>
            {
                last.end = self.written;
                last.ahead = ahead;
                last.full |= full;
                last.first_round &= first_round;
            }
            _ => self.in_flight.push_back(Sent {
                start: self.written - bytes as u64,
                end: self.written,
                at: now,
                ahead,
                full,
                first_round,
                burst: self.burst,
            }),
        }
    }

    /// Back to the floor, growing fast again on the next burst.
    fn forget(&mut self, now: Instant) {
        self.window = LINK_FLOOR;
        self.rates.clear();
        self.startup = true;
        self.last_full = now;
    }

    /// Where the peer's clock stands now, by the lag its reports have shown.
    fn peer_clock(&self, now: Instant) -> f64 {
        now.duration_since(self.epoch).as_secs_f64() - self.min_lag.unwrap_or(0.0)
    }

    /// `read`: when the peer had read them, by its own clock.
    pub(crate) fn received(&mut self, bytes: u64, read: Option<Duration>, now: Instant) {
        self.acked = self.acked.saturating_add(bytes).min(self.written);
        self.last_ack = Some(now);
        let mut newest = None;
        while self.in_flight.front().is_some_and(|s| s.end <= self.acked) {
            newest = self.in_flight.pop_front();
        }
        let Some(sent) = newest else {
            return;
        };
        let now_s = now.duration_since(self.epoch).as_secs_f64();
        let read_s = read.map_or(now_s, |read| read.as_secs_f64());
        let lag = now_s - read_s;
        let raw_rtt = now.duration_since(sent.at);

        let span = self
            .min_rtt
            .unwrap_or(raw_rtt)
            .max(MIN_RATE_SPAN)
            .as_secs_f64();
        while self.acks.len() > 1 && self.acks[1].0 + span <= read_s {
            self.acks.pop_front();
        }
        let (base_read, base_acked) = self.acks.front().copied().unwrap_or((read_s, self.acked));
        let drained = self.acked == self.written;
        // A drained link starts its next rate from its next write, not from before the pause.
        if drained {
            self.acks.clear();
        } else {
            self.acks.push_back((read_s, self.acked));
        }
        let rate = match read_s - base_read {
            elapsed if elapsed > 0.0 => (self.acked - base_acked) as f64 / elapsed,
            _ => 0.0,
        };
        // How far apart the peer read one burst is how fast the link carried it.
        let train = match self.train {
            Some((burst, from, at)) if burst == sent.burst => Some(read_s - at)
                .filter(|&spread| spread >= MIN_SPREAD.as_secs_f64())
                .map(|spread| (sent.end - from) as f64 / spread),
            _ => {
                self.train = Some((sent.burst, sent.end, read_s));
                None
            }
        };

        // What this sample is measured against, before it can move it.
        let bare = self.baseline.filter(|_| self.baseline_confirmed);
        if let Some(started) = self.probe {
            if sent.at >= started && sent.ahead <= 2 * PROBE_BYTES {
                // A probe measures the whole path afresh, the way back included; on a slow
                // link its one part's crossing comes with it, so it is no baseline.
                self.min_rtt = Some(raw_rtt);
                self.min_lag = Some(lag);
                self.probe = None;
            }
        }
        let min_lag = self.min_lag.map_or(lag, |m| m.min(lag));
        self.min_lag = Some(min_lag);
        // A report held up on its way back, behind an upload, says nothing about this link.
        let held = Duration::from_secs_f64((lag - min_lag).max(0.0));
        let rtt = raw_rtt.saturating_sub(held);
        let min_rtt = self.min_rtt.map_or(rtt, |m| m.min(rtt));
        self.min_rtt = Some(min_rtt);
        if sent.ahead <= BASELINE_BYTES {
            if let Some(baseline) = self.baseline {
                self.baseline_confirmed |= rtt <= baseline * 5 / 4 + Duration::from_millis(2);
            }
            self.baseline = Some(self.baseline.map_or(rtt, |b| b.min(rtt)));
        }
        if rtt <= min_rtt * 8 / 5 + Duration::from_millis(2) {
            self.near_min_at = Some(now);
        } else if self.probe.is_none()
            && self
                .near_min_at
                .is_some_and(|at| now.duration_since(at) >= PROBE_AFTER)
        {
            self.probe = Some(now);
        }

        let queued = rtt > min_rtt * 5 / 4 + Duration::from_millis(4);
        // Queued well past what this window allows: the link slowed, so its old peak is gone.
        let draining = rtt > min_rtt * 3 + Duration::from_millis(10);
        let peak_span = (min_rtt * PEAK_ROUNDS).max(MIN_PEAK_SPAN);
        if draining {
            self.rates.clear();
        }
        while self
            .rates
            .front()
            .is_some_and(|&(at, _)| at + peak_span <= now)
        {
            self.rates.pop_front();
        }
        let mut sample = rate.max(train.unwrap_or(0.0));
        // A burst on an empty link that the peer read at once: how late it came past the
        // bare round trip, which two small frames must have agreed on. One that beat it by
        // less than `MIN_SPREAD` is jitter on a link that carried the burst at once.
        if self.startup && !queued && sent.first_round && sent.ahead > BASELINE_BYTES {
            if let Some(bare) = bare.filter(|&bare| rtt + MIN_SPREAD >= bare) {
                let spread = rtt.saturating_sub(bare).max(MIN_SPREAD).as_secs_f64();
                sample = sample.max(sent.ahead as f64 / spread);
            }
        }
        self.rates.push_back((now, sample));
        let peak = self.rates.iter().map(|&(_, r)| r).fold(0.0, f64::max);
        let rtt_s = min_rtt.as_secs_f64();
        let mut target = if draining { rate } else { GAIN * peak } * rtt_s;
        // Grown blind only while the window is what holds the sender back.
        if self.startup && !queued && sent.full {
            target = target.max(STARTUP_GAIN * rate * rtt_s);
        }
        let target = (target as u64).clamp(LINK_FLOOR, LINK_MAX);
        if sent.full {
            self.last_full = now;
        }
        if now.duration_since(self.last_full) >= IDLE {
            // A sender that never fills the window can't show it is still right.
            self.forget(now);
        } else if draining || target > self.window || sent.full {
            // An idle sender's samples can raise the window, never lower it, unless the link slowed.
            self.window = target;
        }
        if sent.full && queued {
            self.startup = false;
        }
        self.reported = !drained;
        if drained {
            self.startup = true;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const MB: f64 = 1e6;
    const UNIT: u64 = 16 * 1024;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// One bottleneck: writes queue for `rate`, then take `one_way` each way. Reports come
    /// back in order, `late` later still, as behind an upload on the uplink.
    struct Sim {
        w: LinkWindow,
        base: Instant,
        t: Duration,
        rate: f64,
        one_way: Duration,
        free_at: Duration,
        last_report: Duration,
    }

    struct Run {
        took: Duration,
        /// The longest a frame written meanwhile would queue at the bottleneck.
        worst_queue: Duration,
    }

    impl Sim {
        fn new(rate: f64, rtt: Duration) -> Self {
            Self {
                w: LinkWindow::new(),
                base: Instant::now(),
                t: Duration::ZERO,
                rate,
                one_way: rtt / 2,
                free_at: Duration::ZERO,
                last_report: Duration::ZERO,
            }
        }

        fn send(&mut self, bytes: u64, waiting: bool, late: Duration) -> (Duration, Duration) {
            self.w.wrote(bytes as usize, waiting, self.base + self.t);
            let done = self.free_at.max(self.t) + Duration::from_secs_f64(bytes as f64 / self.rate);
            self.free_at = done;
            let read = done + self.one_way;
            let at = (read + self.one_way + late).max(self.last_report);
            self.last_report = at;
            (at, read)
        }

        fn bulk(&mut self, total: u64, late: Duration) -> Run {
            let start = self.t;
            let mut sent = 0;
            let mut worst = Duration::ZERO;
            let mut reports = VecDeque::new();
            loop {
                while sent < total && self.w.has_room() {
                    let bytes = UNIT.min(total - sent);
                    sent += bytes;
                    worst = worst.max(self.free_at.saturating_sub(self.t));
                    let (at, read) = self.send(bytes, sent < total, late);
                    reports.push_back((at, read, bytes));
                }
                let Some((at, read, bytes)) = reports.pop_front() else {
                    break;
                };
                self.t = self.t.max(at);
                self.w.received(bytes, Some(read), self.base + self.t);
            }
            Run {
                took: self.t - start,
                worst_queue: worst,
            }
        }

        /// One small frame alone, its report `late` past the bare round trip.
        fn small(&mut self, late: Duration) {
            let (at, read) = self.send(300, false, late);
            self.t = at;
            self.w.received(300, Some(read), self.base + self.t);
        }

        fn idle(&mut self, gap: Duration) {
            self.t += gap;
        }
    }

    #[test]
    fn a_path_that_grows_longer_regains_its_pace() {
        let mut moved = Sim::new(10.0 * MB, ms(4));
        moved.small(Duration::ZERO);
        moved.bulk(20_000_000, Duration::ZERO);
        moved.one_way = ms(75);
        let after = moved.bulk(40_000_000, Duration::ZERO).took;
        let mut fresh = Sim::new(10.0 * MB, ms(150));
        fresh.small(Duration::ZERO);
        let fresh = fresh.bulk(40_000_000, Duration::ZERO).took;
        assert!(
            after < fresh * 2,
            "after the move {after:?}, a fresh connection {fresh:?}"
        );
    }

    #[test]
    fn a_slow_first_small_frame_does_not_flood_a_slow_link() {
        for late in [ms(20), ms(40)] {
            let mut sim = Sim::new(MB, ms(100));
            sim.small(late);
            sim.idle(ms(100));
            let run = sim.bulk(8_000_000, Duration::ZERO);
            assert!(
                run.worst_queue < Duration::from_secs(1),
                "{late:?} late: queued {:?}",
                run.worst_queue
            );
        }
    }

    #[test]
    fn a_window_a_trickle_kept_open_does_not_outlast_a_rate_drop() {
        let mut sim = Sim::new(50.0 * MB, ms(20));
        sim.small(Duration::ZERO);
        sim.bulk(50_000_000, Duration::ZERO);
        for _ in 0..150 {
            sim.idle(ms(200));
            sim.bulk(2000, Duration::ZERO);
        }
        sim.rate = MB;
        sim.idle(ms(500));
        let run = sim.bulk(10_000_000, Duration::ZERO);
        assert!(
            run.worst_queue < Duration::from_secs(1),
            "queued {:?}",
            run.worst_queue
        );
    }

    #[test]
    fn reports_held_up_on_the_uplink_do_not_shrink_the_window() {
        let mut sim = Sim::new(4.0 * MB, ms(50));
        sim.small(Duration::ZERO);
        sim.bulk(20_000_000, Duration::ZERO);
        let before = sim.w.window;
        sim.bulk(20_000_000, ms(500));
        assert!(
            sim.w.window * 2 >= before,
            "{} before, {} with late reports",
            before,
            sim.w.window
        );
    }

    #[test]
    fn a_rate_drop_still_cuts_the_window() {
        let mut sim = Sim::new(4.0 * MB, ms(100));
        sim.small(Duration::ZERO);
        sim.bulk(20_000_000, Duration::ZERO);
        sim.rate = 256.0 * 1024.0;
        let run = sim.bulk(4_000_000, Duration::ZERO);
        // What the old window put on the link drains first; nothing more piles on behind it.
        let bound = Duration::from_secs_f64(GAIN * 4.0 * MB * 0.1 / (256.0 * 1024.0));
        assert!(
            run.worst_queue <= bound + ms(200),
            "queued {:?}, bound {bound:?}",
            run.worst_queue
        );
    }
}
