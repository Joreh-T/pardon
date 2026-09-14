//! pardond 守护进程入口。Task 10 充实（HTTP 触发口 + 剪贴板监听 + 信号处理）。

// Task 7 占位：state/handler 尚未被 main 消费，暂允许 dead_code；Task 10 接线后删除。
#[allow(dead_code)]
mod clip;
#[allow(dead_code)]
mod handler;
#[allow(dead_code)]
mod state;
// 测试假实现共享模块（Task 9 提升）。
#[cfg(test)]
mod testing;

fn main() {
    eprintln!("pardond: not yet wired (Task 10)");
}
