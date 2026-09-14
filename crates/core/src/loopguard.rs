//! 防回环与同内容去重（spec §5.5）：daemon 记录自写剪贴板内容哈希+时间窗，
//! 命中即忽略；连续复制相同文本只触发一次。

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

pub struct LoopGuard {
    window: Duration,
    /// 最近一次被处理的事件（哈希 + 时刻）：连续同内容去重。
    last_seen: Option<(u64, Instant)>,
    /// 自己写入剪贴板的内容（哈希 + 时刻）：回声忽略。
    self_writes: VecDeque<(u64, Instant)>,
}

impl LoopGuard {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            last_seen: None,
            self_writes: VecDeque::new(),
        }
    }

    fn hash(text: &str) -> u64 {
        // DefaultHasher 仅进程内使用（去重/回声），无需跨进程稳定
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        h.finish()
    }

    /// 事件是否应处理。空文本 / 窗口内同内容 / 窗口内自写回声 → false。
    /// 哈希与去重均按 trim 后文本（剪贴板常带尾部换行）。
    pub fn admit(&mut self, text: &str, now: Instant) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        let h = Self::hash(text);
        if let Some((seen, at)) = &self.last_seen {
            if *seen == h && now.duration_since(*at) < self.window {
                return false;
            }
        }
        if self
            .self_writes
            .iter()
            .any(|(w, at)| *w == h && now.duration_since(*at) < self.window)
        {
            return false;
        }
        self.last_seen = Some((h, now));
        true
    }

    /// 写入剪贴板前登记（随后监听到的同一内容即回声，忽略）。
    /// 顺带清理已过期的登记，防止无限增长。
    pub fn register_write(&mut self, text: &str, now: Instant) {
        let h = Self::hash(text.trim());
        while let Some((_, at)) = self.self_writes.front() {
            if now.duration_since(*at) >= self.window {
                self.self_writes.pop_front();
            } else {
                break;
            }
        }
        self.self_writes.push_back((h, now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const W: Duration = Duration::from_secs(10);

    #[test]
    fn empty_and_whitespace_never_admitted() {
        let mut g = LoopGuard::new(W);
        let t = Instant::now();
        assert!(!g.admit("", t));
        assert!(!g.admit("   \n\t", t));
    }

    #[test]
    fn same_text_within_window_is_deduped() {
        let mut g = LoopGuard::new(W);
        let t = Instant::now();
        assert!(g.admit("hello", t));
        assert!(!g.admit("hello", t + Duration::from_secs(3)));
        // 尾部换行视作同一内容
        assert!(!g.admit("hello\n", t + Duration::from_secs(4)));
    }

    #[test]
    fn same_text_after_window_is_readmitted() {
        let mut g = LoopGuard::new(W);
        let t = Instant::now();
        assert!(g.admit("hello", t));
        assert!(g.admit("hello", t + W));
    }

    #[test]
    fn different_text_admitted_immediately() {
        let mut g = LoopGuard::new(W);
        let t = Instant::now();
        assert!(g.admit("hello", t));
        assert!(g.admit("world", t));
    }

    #[test]
    fn self_written_echo_is_ignored() {
        let mut g = LoopGuard::new(W);
        let t = Instant::now();
        assert!(g.admit("hello", t));
        // 译文写回剪贴板 → 监听到的回声应被忽略
        g.register_write("你好", t);
        assert!(!g.admit("你好", t + Duration::from_secs(1)));
        // 其他文本不受影响
        assert!(g.admit("world", t + Duration::from_secs(1)));
    }

    #[test]
    fn self_write_echo_expires_with_window() {
        let mut g = LoopGuard::new(W);
        let t = Instant::now();
        g.register_write("你好", t);
        assert!(!g.admit("你好", t + Duration::from_secs(9)));
        assert!(g.admit("你好", t + W));
    }

    #[test]
    fn trim_normalizes_before_hashing() {
        let mut g = LoopGuard::new(W);
        let t = Instant::now();
        g.register_write("  你好 \n", t);
        assert!(!g.admit("你好", t), "登记与判定都按 trim 后文本");
    }
}
