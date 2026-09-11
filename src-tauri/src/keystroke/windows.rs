use anyhow::anyhow;
use std::mem::{size_of, zeroed};
use winapi::um::winuser::{
    SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL, VK_SHIFT,
};

use crate::core::error::Result;

/// V 键的虚拟键码，winapi 未直接导出。
const VK_V: u16 = 0x56;

/// 向系统事件队列投递一次完整按键序列（按下 + 抬起）。
///
/// `shift_down` 为 true 时包一层 Shift，供搜索回放保留用户实际按下的 Shift 状态
/// （大写字母 / 符号键位），否则回放出来的字符会丢失大小写。
pub fn send_keystroke(vk: u16, shift_down: bool) -> Result<()> {
    let mut inputs: [INPUT; 4] = unsafe { zeroed() };
    let mut len = 0;

    if shift_down {
        fill_keyboard_input(&mut inputs[len], VK_SHIFT as u16, 0);
        len += 1;
    }

    fill_keyboard_input(&mut inputs[len], vk, 0);
    fill_keyboard_input(&mut inputs[len + 1], vk, KEYEVENTF_KEYUP);
    len += 2;

    if shift_down {
        fill_keyboard_input(&mut inputs[len], VK_SHIFT as u16, KEYEVENTF_KEYUP);
        len += 1;
    }

    let sent = unsafe { SendInput(len as u32, inputs.as_mut_ptr(), size_of::<INPUT>() as i32) };
    if sent as usize != len {
        return Err(anyhow!("SendInput injected {sent}/{len} events").into());
    }
    Ok(())
}

fn fill_keyboard_input(input: &mut INPUT, vk: u16, flags: u32) {
    input.type_ = INPUT_KEYBOARD;
    unsafe {
        *input.u.ki_mut() = KEYBDINPUT {
            wVk: vk,
            wScan: 0,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        };
    }
}

/// 向系统事件队列投递一次 Ctrl+V，模拟「粘贴」。
///
/// 之所以选 Ctrl+V 而非 Shift+Insert：
/// - Monaco editor（VS Code 聊天输入、所有 Web 嵌入式代码编辑器）把 Insert 解释为
///   「切换插入/改写模式」并吞掉 Shift 修饰，导致「粘贴失效 + 光标变成块状」。
/// - Ctrl+V 是 Windows / Chromium / Electron / 现代 IDE 输入控件普遍约定，
///   与剪贴板内容类型无关，覆盖面最广。
pub fn simulate_paste() -> Result<()> {
    let mut inputs: [INPUT; 4] = unsafe { zeroed() };

    fill_keyboard_input(&mut inputs[0], VK_CONTROL as u16, 0);
    fill_keyboard_input(&mut inputs[1], VK_V, 0);
    fill_keyboard_input(&mut inputs[2], VK_V, KEYEVENTF_KEYUP);
    fill_keyboard_input(&mut inputs[3], VK_CONTROL as u16, KEYEVENTF_KEYUP);

    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_mut_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent as usize != inputs.len() {
        return Err(anyhow!("SendInput injected {sent}/{} events", inputs.len()).into());
    }
    Ok(())
}
