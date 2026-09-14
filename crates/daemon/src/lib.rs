//! pardon-daemon 库面：供集成测试（tests/）与 main 复用。

pub mod clip;
pub mod handler;
pub mod http;
pub mod state;
/// 测试假实现。常驻编译（不能 cfg(test)——集成测试以普通依赖编译 lib，
/// cfg(test) 模块对外不可见）；doc(hidden) 使其不出现在文档。
#[doc(hidden)]
pub mod testing;
