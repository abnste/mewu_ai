# 喵呜AI / MewuAI 0.7.4

## 简体中文

- 修复 0.7.3 截图框选不跟手的问题：拖动选区时不再重复刷新放大镜，也不再对固定尺寸的放大镜卡片逐次强制同步排版；尺寸与坐标读数继续实时更新。
- 更新器为未来正式发布的 1.0 重制版做好兼容：用户在软件内确认更新后，可下载、校验并启动重制版安装程序。0.7.x 沿用现有安装方式；重制版采用对应的 NSIS 安装参数。重制版尚未正式发布，本版不包含重制版软件或数据迁移。

## English

- Fix sluggish capture selection introduced in 0.7.3. Dragging no longer refreshes the magnifier twice or forces synchronous layout of its fixed-size card on every pointer move. Dimensions and coordinates still update as the selection changes.
- Prepare the updater for a future stable 1.0 remake. After confirmation in the app, it can download and verify the remake installer and start installation with the appropriate NSIS arguments. Existing 0.7.x updates keep their current installer behavior. The remake has not been released yet; this package does not include it or migrate user data.

## 下载 / Downloads

[安装版 EXE / Installer EXE](https://github.com/abnste/mewu_ai/releases/download/v0.7.4/MewuAI-Setup-0.7.4-win-x64.exe) · [便携版 ZIP / Portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.7.4/MewuAI-Portable-0.7.4-win-x64.zip)

安装包和便携包使用 GitHub 资产自带的 SHA-256 校验值，不额外附带校验文件。 / The installer and portable package use GitHub's built-in SHA-256 asset digests; no separate checksum file is attached.

[完整更新历史 / Full changelog](https://github.com/abnste/mewu_ai/blob/v0.7.4/CHANGELOG.md)
