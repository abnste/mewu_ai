// SPDX-License-Identifier: MPL-2.0
//! Read-only key/button observation, separate from Enigo's input submission.
use super::*;
use x11rb::{
    connection::Connection, protocol::xproto::ConnectionExt, rust_connection::RustConnection,
};
pub(super) struct PhysicalInput {
    connection: RustConnection,
    root: u32,
    escape: Vec<u8>,
}
impl PhysicalInput {
    pub fn new() -> Result<Self> {
        let (connection, index) = x11rb::connect(None).map_err(|_| "无法读取 X11 键鼠状态")?;
        let setup = connection.setup();
        let root = setup.roots.get(index).ok_or("X11 屏幕不可用")?.root;
        let min = setup.min_keycode;
        let count = setup
            .max_keycode
            .checked_sub(min)
            .and_then(|v| v.checked_add(1))
            .ok_or("X11 键位范围无效")?;
        let mapping = connection
            .get_keyboard_mapping(min, count)
            .map_err(|_| "无法读取键位")?
            .reply()
            .map_err(|_| "无法读取键位")?;
        let width = usize::from(mapping.keysyms_per_keycode);
        if width == 0 {
            return Err("X11 键位无效".into());
        }
        let escape = mapping
            .keysyms
            .chunks(width)
            .enumerate()
            .filter(|(_, syms)| syms.contains(&0xff1b))
            .filter_map(|(index, _)| u8::try_from(usize::from(min) + index).ok())
            .collect::<Vec<_>>();
        if escape.is_empty() {
            return Err("无法读取 Esc 停止键".into());
        }
        Ok(Self {
            connection,
            root,
            escape,
        })
    }
    pub fn neutral(&self) -> Result<bool> {
        let pointer = self
            .connection
            .query_pointer(self.root)
            .map_err(|_| "无法读取键鼠状态")?
            .reply()
            .map_err(|_| "无法读取键鼠状态")?;
        let mask = u16::from(pointer.mask);
        Ok(
            mask & ((1 << 0) | (1 << 2) | (1 << 3) | (1 << 6) | (1 << 8) | (1 << 9) | (1 << 10))
                == 0,
        )
    }
    pub fn escape_down(&self) -> Result<bool> {
        let map = self
            .connection
            .query_keymap()
            .map_err(|_| "无法读取 Esc 状态")?
            .reply()
            .map_err(|_| "无法读取 Esc 状态")?;
        Ok(self
            .escape
            .iter()
            .any(|code| map.keys[usize::from(*code) / 8] & (1 << (*code % 8)) != 0))
    }
}
