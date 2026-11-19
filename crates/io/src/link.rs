//! Ableton Link。映像ソフトやDAWとテンポ・拍を共有する。
//!
//! Link のコードは GPL なので、このアプリには入れていない。
//! 代わりに、別のプログラムの Carabiner (https://github.com/Deep-Symmetry/carabiner) に、同じPC内の TCP でつなぐ。
//! Carabiner が Link に参加し、こちらはテンポと拍の位置を聞いたり、変えてもらったりするだけ。
//! Beat Link Trigger も Carabiner を使うので、同じ Carabiner に一緒につなげる。

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Carabiner の既定のポート
pub const DEFAULT_PORT: u16 = 17000;
/// Carabiner が返す拍の位置は、1小節4拍で数えたもの
const QUANTUM: f64 = 4.0;
/// `status` を聞く間隔。拍の位置は、間の時間をこちらで補って出す
const POLL: Duration = Duration::from_millis(100);
/// この時間、Carabiner から何も届かなければ、つながっていない扱いにする
const STALE: Duration = Duration::from_millis(1500);
/// こちらが変えた直後は、変える前に出された古い状態が届くことがある。その間は受け取らない
const HOLD: Duration = Duration::from_millis(60);
const BPM_RANGE: (f64, f64) = (20.0, 999.0);

/// テストで接続先を変えるためのポート。0 なら環境変数か既定を使う
static PORT_OVERRIDE: AtomicU16 = AtomicU16::new(0);

/// テストで接続先を変える。ポートは全体で1つなので、返す鍵を持っている間だけ使う
#[cfg(any(test, feature = "test-support"))]
pub fn set_test_port(p: u16) -> MutexGuard<'static, ()> {
    static KEY: Mutex<()> = Mutex::new(());
    let g = lock(&KEY);
    PORT_OVERRIDE.store(p, Ordering::Relaxed);
    g
}

/// テストでは、このPCに入れてある Carabiner を勝手に起動しない
fn autostart_allowed() -> bool {
    PORT_OVERRIDE.load(Ordering::Relaxed) == 0
}

/// 接続するポート。環境変数 `DMXCTL_CARABINER_PORT` で変えられる
pub fn port() -> u16 {
    let p = PORT_OVERRIDE.load(Ordering::Relaxed);
    if p != 0 {
        return p;
    }
    std::env::var("DMXCTL_CARABINER_PORT").ok().and_then(|s| s.trim().parse().ok()).filter(|p| *p != 0).unwrap_or(DEFAULT_PORT)
}

/// Carabiner が教えてくれた、ある時点の状態
#[derive(Clone, Copy, Debug, PartialEq)]
struct Snap {
    /// 受け取った時刻
    at: Instant,
    peers: u64,
    bpm: f64,
    /// Link の時計で、0拍目の時刻。マイクロ秒
    start: i64,
    beat: f64,
}

impl Snap {
    /// `now` の拍の位置。受け取ってからの時間を、テンポで進めて補う
    fn beat_at(&self, now: Instant) -> f64 {
        self.beat + now.saturating_duration_since(self.at).as_secs_f64() * self.bpm / 60.0
    }
}

#[derive(Default)]
struct Shared {
    snap: Option<Snap>,
    /// この時刻までは、届いた状態を捨てる
    hold_until: Option<Instant>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 有効にしている間、Carabiner を通して、Link対応アプリとテンポ・拍の位置が揃う。
/// Carabiner が動いていないときは、つながっていない扱いで、何も起きない
pub struct Link {
    shared: Arc<Mutex<Shared>>,
    tx: Mutex<mpsc::Sender<String>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Link {
    /// Carabiner につなぐ。つながるまでは裏で繰り返し試す。`bpm` は、仲間がいないときだけ使う最初のテンポ
    pub fn start(bpm: f64) -> Option<Link> {
        Link::connect(port(), bpm)
    }

    fn connect(port: u16, bpm: f64) -> Option<Link> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (s, st) = (shared.clone(), stop.clone());
        let thread = std::thread::Builder::new().name("link".into()).spawn(move || run(port, bpm, s, rx, st)).ok()?;
        Some(Link { shared, tx: Mutex::new(tx), stop, thread: Some(thread) })
    }

    fn fresh(&self) -> Option<Snap> {
        lock(&self.shared).snap.filter(|s| s.at.elapsed() < STALE)
    }

    fn send(&self, line: String) {
        let _ = lock(&self.tx).send(line);
    }

    /// Carabiner につながっているか
    pub fn connected(&self) -> bool {
        self.fresh().is_some()
    }

    /// Link でつながっている仲間の数
    pub fn peers(&self) -> u64 {
        self.fresh().map_or(0, |s| s.peers)
    }

    /// 今のテンポ(BPM)と、1小節4拍で数えた拍の位置。Carabiner につながっていないときは None
    pub fn state(&self) -> Option<(f64, f64)> {
        self.fresh().map(|s| (s.bpm, s.beat_at(Instant::now())))
    }

    pub fn set_tempo(&self, bpm: f64) {
        let bpm = bpm.clamp(BPM_RANGE.0, BPM_RANGE.1);
        {
            let mut g = lock(&self.shared);
            let now = Instant::now();
            if let Some(s) = g.snap.as_mut() {
                s.beat = s.beat_at(now);
                s.at = now;
                s.bpm = bpm;
                g.hold_until = Some(now + HOLD);
            }
        }
        self.send(format!("bpm {bpm:.3}\n"));
    }

    /// 今この瞬間を`beat`拍目にする。頭出しやTAPに使う。仲間の拍の位置も一緒に動く
    pub fn force_beat(&self, beat: f64) {
        let when = {
            let mut g = lock(&self.shared);
            let now = Instant::now();
            let Some(s) = g.snap.as_mut() else { return };
            // 今の Link の時計は、0拍目の時刻に、そこからの拍数ぶんの時間を足せば出る
            let when = s.start + (s.beat_at(now) * 60e6 / s.bpm).round() as i64;
            s.start = when - (beat * 60e6 / s.bpm).round() as i64;
            s.beat = beat;
            s.at = now;
            g.hold_until = Some(now + HOLD);
            when
        };
        self.send(format!("force-beat-at-time {beat:.6} {when} {QUANTUM}\n"));
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// つなぎ直しながら、止められるまで動く裏のスレッド
fn run(port: u16, initial_bpm: f64, shared: Arc<Mutex<Shared>>, rx: mpsc::Receiver<String>, stop: Arc<AtomicBool>) {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    // 入れてある Carabiner を、このアプリが起動したとき、その子プロセス
    let mut child: Option<std::process::Child> = None;
    let mut spawn_tries = 0;
    while !stop.load(Ordering::Relaxed) {
        match TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
            Ok(stream) => {
                // つながっていない間にたまった操作は、古いので捨てる
                while rx.try_recv().is_ok() {}
                session(stream, initial_bpm, &shared, &rx, &stop);
                *lock(&shared) = Shared::default();
            }
            Err(_) => {
                while rx.try_recv().is_ok() {}
                // 誰もつながる先を用意していなければ、入れてある Carabiner を起動する。既に誰かが動かしていれば、その Carabiner につなぐので、ここには来ない
                if child.as_mut().is_some_and(|c| c.try_wait().ok().flatten().is_some()) {
                    child = None;
                }
                if child.is_none() && spawn_tries < 3 && autostart_allowed() {
                    child = crate::carabiner::spawn(port);
                    if child.is_some() {
                        spawn_tries += 1;
                    }
                }
            }
        }
        // 次の接続まで1秒待つ。終了の合図には素早く応じる
        for _ in 0..50 {
            if stop.load(Ordering::Relaxed) {
                if let Some(mut c) = child.take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// 1回のつながりの間、状態を聞き続ける。切れたら戻る
fn session(mut stream: TcpStream, initial_bpm: f64, shared: &Mutex<Shared>, rx: &mpsc::Receiver<String>, stop: &AtomicBool) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    let mut inbox = Vec::new();
    let mut chunk = [0u8; 1024];
    let mut last_poll: Option<Instant> = None;
    let mut first = true;
    while !stop.load(Ordering::Relaxed) {
        while let Ok(line) = rx.try_recv() {
            if stream.write_all(line.as_bytes()).is_err() {
                return;
            }
        }
        if last_poll.is_none_or(|t| t.elapsed() >= POLL) {
            if stream.write_all(b"status\n").is_err() {
                return;
            }
            last_poll = Some(Instant::now());
        }
        match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => inbox.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
            Err(_) => return,
        }
        while let Some(i) = inbox.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = inbox.drain(..=i).collect();
            let Some(mut snap) = parse_status(&String::from_utf8_lossy(&line)) else { continue };
            let mut g = lock(shared);
            if g.hold_until.is_some_and(|t| Instant::now() < t) {
                continue;
            }
            snap.at = Instant::now();
            g.snap = Some(snap);
            drop(g);
            // 最初につながったとき、仲間がいなければ、こちらのテンポで始める。いれば仲間に合わせる
            if first {
                first = false;
                if snap.peers == 0 && (snap.bpm - initial_bpm).abs() > 0.01 {
                    let bpm = initial_bpm.clamp(BPM_RANGE.0, BPM_RANGE.1);
                    if stream.write_all(format!("bpm {bpm:.3}\n").as_bytes()).is_err() {
                        return;
                    }
                }
            }
        }
    }
}

/// `status { :peers 0 :bpm 120.000000 :start 73743731220 :beat 597.737570 }` を読む
fn parse_status(line: &str) -> Option<Snap> {
    let body = line.trim().strip_prefix("status")?.trim().strip_prefix('{')?.strip_suffix('}')?;
    let (mut peers, mut bpm, mut start, mut beat) = (None, None, None, None);
    let mut it = body.split_whitespace();
    while let Some(key) = it.next() {
        let Some(val) = it.next() else { break };
        match key {
            ":peers" => peers = val.parse::<u64>().ok(),
            ":bpm" => bpm = val.parse::<f64>().ok(),
            ":start" => start = val.parse::<i64>().ok(),
            ":beat" => beat = val.parse::<f64>().ok(),
            _ => {}
        }
    }
    let bpm = bpm.filter(|b| *b > 0.0)?;
    Some(Snap { at: Instant::now(), peers: peers?, bpm, start: start?, beat: beat? })
}

/// テスト用の、Carabiner のふりをするサーバー
#[cfg(any(test, feature = "test-support"))]
pub mod fake {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    struct State {
        bpm: f64,
        peers: u64,
        /// `t0` の時点の拍
        beat0: f64,
        t0: Instant,
        /// 受けた force-beat-at-time の、`when` と本当の「今」の差。マイクロ秒
        forced: Vec<(f64, i64)>,
    }

    pub struct FakeCarabiner {
        pub port: u16,
        st: Arc<Mutex<State>>,
    }

    const CLOCK_BASE: i64 = 5_000_000_000;

    impl FakeCarabiner {
        pub fn new(bpm: f64, peers: u64) -> FakeCarabiner {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let epoch = Instant::now();
            let st = Arc::new(Mutex::new(State { bpm, peers, beat0: 37.0, t0: epoch, forced: Vec::new() }));
            let st2 = st.clone();
            std::thread::spawn(move || {
                for conn in listener.incoming() {
                    let Ok(conn) = conn else { break };
                    let st = st2.clone();
                    std::thread::spawn(move || serve(conn, st, epoch));
                }
            });
            FakeCarabiner { port, st }
        }

        /// 仲間がテンポを変えたことにする
        pub fn peer_set_bpm(&self, bpm: f64) {
            rebase(&mut lock(&self.st), Instant::now()).bpm = bpm;
        }

        pub fn bpm(&self) -> f64 {
            lock(&self.st).bpm
        }

        pub fn beat(&self) -> f64 {
            beat_now(&lock(&self.st), Instant::now())
        }

        pub fn forced(&self) -> Vec<(f64, i64)> {
            lock(&self.st).forced.clone()
        }
    }

    fn beat_now(s: &State, now: Instant) -> f64 {
        s.beat0 + now.duration_since(s.t0).as_secs_f64() * s.bpm / 60.0
    }

    fn rebase(s: &mut State, now: Instant) -> &mut State {
        s.beat0 = beat_now(s, now);
        s.t0 = now;
        s
    }

    fn status(s: &State, epoch: Instant) -> String {
        let now = Instant::now();
        let beat = beat_now(s, now);
        let clock = CLOCK_BASE + now.duration_since(epoch).as_micros() as i64;
        let start = clock - (beat * 60e6 / s.bpm).round() as i64;
        format!("status {{ :peers {} :bpm {:.6} :start {start} :beat {beat:.6} }}\n", s.peers, s.bpm)
    }

    fn serve(conn: TcpStream, st: Arc<Mutex<State>>, epoch: Instant) {
        let mut out = conn.try_clone().unwrap();
        for line in BufReader::new(conn).lines() {
            let Ok(line) = line else { return };
            let mut w = line.split_whitespace();
            let reply = {
                let mut s = lock(&st);
                match w.next() {
                    Some("status") => Some(status(&s, epoch)),
                    Some("bpm") => {
                        if let Some(b) = w.next().and_then(|v| v.parse().ok()) {
                            rebase(&mut s, Instant::now()).bpm = b;
                        }
                        Some(status(&s, epoch))
                    }
                    Some("force-beat-at-time") => {
                        let a: Vec<f64> = w.filter_map(|v| v.parse().ok()).collect();
                        if let [beat, when, _q] = a[..] {
                            let now = Instant::now();
                            let clock = CLOCK_BASE + now.duration_since(epoch).as_micros() as i64;
                            s.forced.push((beat, when as i64 - clock));
                            s.beat0 = beat;
                            s.t0 = now;
                        }
                        Some(status(&s, epoch))
                    }
                    _ => None,
                }
            };
            if let Some(r) = reply
                && out.write_all(r.as_bytes()).is_err()
            {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeCarabiner;
    use super::*;

    fn wait(cond: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cond() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        cond()
    }

    #[test]
    fn parses_status_lines() {
        let s = parse_status("status { :peers 2 :bpm 128.500000 :start 73743731220 :beat 597.737570 }\n").unwrap();
        assert_eq!((s.peers, s.bpm, s.start, s.beat), (2, 128.5, 73743731220, 597.73757));
        // 転送の状態が付いていても読める
        assert!(parse_status("status { :peers 0 :bpm 120.0 :start 1 :beat 2.0 :playing true }").is_some());
        // 状態ではない返事や、欠けた行は読まない
        assert!(parse_status("beat-at-time { :when 1 :quantum 4.0 :beat 5.25 }").is_none());
        assert!(parse_status("status { :peers 0 :bpm 120.0 }").is_none());
        assert!(parse_status("bad-bpm ").is_none());
    }

    #[test]
    fn not_connected_without_carabiner() {
        // 誰も待っていないポートを選ぶ
        let port = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        let l = Link::connect(port, 120.0).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(!l.connected());
        assert_eq!(l.peers(), 0);
        assert!(l.state().is_none());
        // つながっていなくても、操作で落ちない
        l.set_tempo(100.0);
        l.force_beat(0.0);
    }

    #[test]
    fn follows_carabiner_and_sends_changes() {
        let fake = FakeCarabiner::new(100.0, 1);
        let l = Link::connect(fake.port, 100.0).unwrap();
        assert!(wait(|| l.connected()), "Carabiner につながらない");
        assert_eq!(l.peers(), 1);
        let (t, b0) = l.state().unwrap();
        assert!((t - 100.0).abs() < 1e-6);
        std::thread::sleep(Duration::from_millis(300));
        // 100BPM で 0.3秒 = 0.5拍。状態は0.1秒おきにしか来ないので、間は補われている
        let (_, b1) = l.state().unwrap();
        assert!((b1 - b0 - 0.5).abs() < 0.1, "{b0} -> {b1}");
        // 実際の拍の位置とも揃っている
        assert!((b1 - fake.beat()).abs() < 0.1, "{b1} / {}", fake.beat());

        // 仲間がテンポを変えると、こちらも追従する
        fake.peer_set_bpm(140.0);
        assert!(wait(|| (l.state().unwrap().0 - 140.0).abs() < 1e-3));
        // こちらのテンポ変更は、すぐ読み取れ、Carabiner にも伝わる
        l.set_tempo(90.0);
        assert!((l.state().unwrap().0 - 90.0).abs() < 1e-6);
        assert!(wait(|| (fake.bpm() - 90.0).abs() < 1e-3));
        std::thread::sleep(Duration::from_millis(250));
        assert!((l.state().unwrap().0 - 90.0).abs() < 1e-6, "古い状態で戻された");

        // 頭出しは、Link の時計の「今」を付けて頼む
        l.force_beat(0.0);
        assert!(l.state().unwrap().1.abs() < 0.05);
        assert!(wait(|| !fake.forced().is_empty()));
        let (beat, off) = fake.forced()[0];
        assert_eq!(beat, 0.0);
        assert!(off.abs() < 30_000, "Link の時計の今とずれている: {off}マイクロ秒");
        assert!(fake.beat().abs() < 1.0);
    }

    #[test]
    fn starts_with_own_tempo_when_alone_and_adopts_peers_tempo() {
        let alone = FakeCarabiner::new(120.0, 0);
        let _l = Link::connect(alone.port, 95.0).unwrap();
        assert!(wait(|| (alone.bpm() - 95.0).abs() < 1e-3), "仲間がいないのに、こちらのテンポで始まらない");

        let busy = FakeCarabiner::new(120.0, 2);
        let l = Link::connect(busy.port, 95.0).unwrap();
        assert!(wait(|| l.connected()));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(busy.bpm(), 120.0, "仲間がいるのに、テンポを書き換えた");
        assert!((l.state().unwrap().0 - 120.0).abs() < 1e-6);
    }
}
