//! 进程内存指标：偏好页诊断面板与内存基准脚本共用。
//!
//! 只读当前 Rust 进程的 OS 级占用（RSS / 虚拟内存），不引入后台采样任务；
//! 想看新数字就重新调一次命令。WebView 进程内存由前端 `performance.memory` 补充。

use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

/// 偏好页诊断面板展示的进程内存占用。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStats {
    /// 物理内存驻留集（Resident Set Size），字节。
    pub rss_bytes: u64,
    /// 虚拟内存提交量，字节；与 RSS 的差值主要是映射未触碰的地址空间。
    pub virtual_bytes: u64,
}

/// 读取当前进程的 RSS 与虚拟内存占用。
pub fn process_memory() -> MemoryStats {
    let pid = sysinfo::get_current_pid().expect("current pid is always valid");

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );

    let Some(process) = system.process(pid) else {
        return MemoryStats {
            rss_bytes: 0,
            virtual_bytes: 0,
        };
    };

    MemoryStats {
        rss_bytes: process.memory(),
        virtual_bytes: process.virtual_memory(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_memory_returns_nonzero_rss_on_current_platform() {
        let stats = process_memory();

        assert!(stats.rss_bytes > 0);
    }
}
