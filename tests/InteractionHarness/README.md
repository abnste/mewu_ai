# 覆盖层交互回放

仅本机 Debug 验收：`dotnet run --project tests/InteractionHarness/InteractionHarness.csproj -c Debug -p:Platform=x64`。
追加 `-- --english` 查看英文布局。Esc 退出，五分钟自动关闭。

回放实际覆盖层的合成长回答，检查输入条向上展开、历史入口、鼠标滚动暂停跟随和“回到最新回复”。背景为合成卡片，不加载真实设置或磁盘历史、不发送网络请求、不启动录屏。仅设置 Debug 已有防捕获 QA 开关；不得加入正式发布。

先退出正常应用再启动，避免和用户正在操作的覆盖层混淆。

运行 `dotnet tests/InteractionHarness/bin/x64/Debug/net10.0-windows10.0.19041.0/InteractionHarness.dll --verify-lifetime` 执行真实覆盖层的资源回收契约检查：创建/删除 60 个区域，在 50 步历史中保留 25 个可撤销区域，验证 redo 与在途请求保护，历史淘汰后确认区域和租约归零。结果仅写入忽略的 `.codex-build/interaction-lifetime.json`，不生成真实媒体文件。

性能回放：Release 构建后以 `--teaching --benchmark` 记录修改前样本，追加 `--after` 记录修改后样本；合成 160 次更新、9010 字正文，结果仅在 `.codex-build/answer-before.json` 和 `answer-after.json`。测量期间不要同时构建、跑测试或做桌面自动化，避免 CPU/GPU 竞争污染结果。该回放展示真实覆盖层并在完成时关闭，不能与用户日常截图混用。

滚动条箭头回归：Release 构建后运行 `dotnet tests/InteractionHarness/bin/x64/Release/net10.0-windows10.0.19041.0/InteractionHarness.dll --teaching --verify-answer-alignment`。真实覆盖层渲染长回答，比较滑块与箭头的实际中心坐标（误差不超过 0.1 DIP），并验证箭头显隐不改变正文及对话条边界；失败返回非零退出码。
