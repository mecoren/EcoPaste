//! 进程内存指标：偏好页诊断面板与内存基准脚本共用。
//!
//! 只读当前 Rust 进程的 OS 级占用（RSS / 虚拟内存），不引入后台采样任务；
//! 想看新数字就重新调一次命令。WebView 进程内存由前端 `performance.memory` 补充。

use serde::{Deserialize, Serialize};

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
    inner::process_memory()
}

#[cfg(target_os = "windows")]
mod inner {
    use super::MemoryStats;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    pub fn process_memory() -> MemoryStats {
        // PageFaultCount 等其余计数器对诊断面板无消费方；填 0 的 MEMORY_COUNTERS
        // 与 cb 字段按 Win32 约定由 GetProcessMemoryInfo 全量回写。
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        let result = unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut counters,
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            )
        };

        if result.is_err() {
            return MemoryStats {
                rss_bytes: 0,
                virtual_bytes: 0,
            };
        }

        MemoryStats {
            rss_bytes: counters.WorkingSetSize as u64,
            virtual_bytes: counters.PagefileUsage as u64,
        }
    }
}

#[cfg(target_os = "macos")]
mod inner {
    use super::MemoryStats;
    use core_foundation::base::mach_port_t;
    use core_foundation::mach::mach_task_basic_info_data_t;

    const MACH_TASK_BASIC_INFO: natural_t = 20;
    const MACH_TASK_BASIC_INFO_COUNT: mach_msg_type_number_t =
        core::mem::size_of::<mach_task_basic_info_data_t>() as mach_msg_type_number_t
            / core::mem::size_of::<natural_t>() as mach_msg_type_number_t;

    pub fn process_memory() -> MemoryStats {
        let mut info = mach_task_basic_info_data_t::default();
        let mut count = MACH_TASK_BASIC_INFO_COUNT;
        let result = unsafe { task_info_wrapper(&mut info, &mut count) };

        if !result {
            return MemoryStats {
                rss_bytes: 0,
                virtual_bytes: 0,
            };
        }

        MemoryStats {
            rss_bytes: info.resident_size,
            virtual_bytes: info.virtual_size,
        }
    }

    /// `mach_task_basic_info` 查询；失败（含 KERN_INVALID_ARGUMENT 之外的罕见
    /// 竞态错误码）一律按「读不到」处理，诊断面板展示 0。
    unsafe fn task_info_wrapper(
        info: *mut mach_task_basic_info_data_t,
        count: *mut mach_msg_type_number_t,
    ) -> bool {
        let result = mach::task_info(
            mach_task_self(),
            MACH_TASK_BASIC_INFO,
            info as *mut natural_t,
            count,
        );

        result == mach::KERN_SUCCESS
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
