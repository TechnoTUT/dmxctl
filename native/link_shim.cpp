// Ableton Link への最小限の窓口(テンポと拍位置だけ)。Rust側は src/link.rs
#include <ableton/Link.hpp>
#include <cstdint>

using ableton::Link;

static Link &L(void *p) { return *static_cast<Link *>(p); }

extern "C" {

void *dmx_link_new(double bpm) {
  try {
    return new Link(bpm);
  } catch (...) {
    return nullptr;
  }
}

void dmx_link_free(void *p) {
  try {
    delete static_cast<Link *>(p);
  } catch (...) {
  }
}

void dmx_link_enable(void *p, bool on) {
  try {
    L(p).enable(on);
  } catch (...) {
  }
}

uint64_t dmx_link_peers(void *p) {
  try {
    return static_cast<uint64_t>(L(p).numPeers());
  } catch (...) {
    return 0;
  }
}

// 今この瞬間のテンポ(BPM)と拍位置。quantum = 1小節の拍数(拍の頭を仲間と揃える単位)
bool dmx_link_state(void *p, double quantum, double *tempo, double *beat) {
  try {
    auto &l = L(p);
    const auto s = l.captureAppSessionState();
    const auto now = l.clock().micros();
    *tempo = s.tempo();
    *beat = s.beatAtTime(now, quantum);
    return true;
  } catch (...) {
    return false;
  }
}

void dmx_link_set_tempo(void *p, double bpm) {
  try {
    auto &l = L(p);
    auto s = l.captureAppSessionState();
    s.setTempo(bpm, l.clock().micros());
    l.commitAppSessionState(s);
  } catch (...) {
  }
}

// 今この瞬間を`beat`拍目にする(頭出し/タップ)。仲間の拍位置も一緒に動く
void dmx_link_force_beat(void *p, double beat, double quantum) {
  try {
    auto &l = L(p);
    auto s = l.captureAppSessionState();
    s.forceBeatAtTime(beat, l.clock().micros(), quantum);
    l.commitAppSessionState(s);
  } catch (...) {
  }
}
}
