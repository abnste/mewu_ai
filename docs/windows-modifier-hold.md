# Windows 修饰键长按传输

`hold_key_windows` 只负责向通用事件触发器传递按下和释放；截图、语音等行为由规则执行。长按阈值来自规则，不修改 Windows 注册表、系统 AI 设置或键盘驱动。

## 两种输入不能混用

- `STATE_TAG` 是清除低层修饰键状态的私有 key-up。钩子必须消费它，不交给后续钩子或应用；否则状态清理可能被系统当成用户抬起 Win，触发开始菜单。该操作没有 key-down，也没有额外的菜单掩码按键。
- `TAG` 仅用于短按和普通组合键的真实重放，必须继续交给系统。短按只重放一次 down/up；在长按被接管前输入其他键或点击/滚动，先还原 Win-down，再让原输入继续。已有其他按键或已按住 Win 时启动传输，保持原生行为。

物理 Win-down 及重复 down 都可能重新断言系统状态。清理在同一消息线程离开键盘回调后执行，并重新核对该键仍属于本次接管，不能延迟清掉已经交还系统的组合键。已接管后的鼠标操作还需在进入下一层鼠标钩子前清理两侧 Win，避免系统把它认作 Win+单击。

接管后的 Escape 和普通操作交给当前 Mewu 界面；Win 的物理释放结束手势。截图关闭或快捷键录入状态变化不能通过重新读取已被清除的系统状态忘掉物理所有权，否则硬件重复消息会制造第二次短按。两侧 Win 分别保持到各自物理释放。

## 验证范围

纯状态回归覆盖短按、阈值、重复、双侧 Win、编辑取消、Escape、组合键顺序及快速鼠标组合。Windows 原生测试直接检查真实钩子回调消费私有状态消息，但不向桌面注入 Win。物理键测试需另外检查长按→单击→松开→Escape、无操作松开、短按及 Win+E/R/D/V/Tab/方向键，不能把状态测试等同系统界面互斥的实键结果。

## 一手参考

- [Microsoft LowLevelKeyboardProc](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc)：回调在异步状态更新前执行，保持短回调与消息循环。
- [PowerToys Keyboard Manager](https://github.com/microsoft/PowerToys/blob/main/doc/devdocs/modules/keyboardmanager/keyboardmanager.md#modifier-caps-lock-interaction-on-japanese-ime-keyboards)：区分低层状态重置与向应用重放输入的原则。Mewu 将该原则用于自己的 Win 长按所有权，不复制其 Caps Lock 重映射业务。
- [Microsoft Click to Do 快捷键](https://support.microsoft.com/en-us/windows/ai/ai-features/keyboard-shortcuts-for-recall-and-click-to-do)：Win+单击本身就是系统功能，不能只测试开始菜单或只吞 Win-up。
