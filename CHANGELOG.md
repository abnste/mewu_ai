# 更新历史 / Changelog

每个正式版本对应自己的源码标签、安装包和双语发行说明。后续功能写入新版本；旧版本说明保留当时发布内容。/ Each release has its own source tag, packages, and bilingual notes. Later changes belong to later releases.

## 0.6.0 — 基于 0.5.10 的录屏兼容修复 / Recording compatibility after 0.5.10

发行说明 / Release notes: [0.6.0](https://github.com/abnste/mewu_ai/releases/tag/v0.6.0)

完整的双语发行说明见 [docs/release-notes-v0.6.0.md](./docs/release-notes-v0.6.0.md)。 / See the bilingual notes in [docs/release-notes-v0.6.0.md](./docs/release-notes-v0.6.0.md).

## 0.5.10 — 0.5.3 以来的累计更新与录屏声音稳定性 / Cumulative updates since 0.5.3 and recording-audio stability

发行说明 / Release notes: [0.5.10](https://github.com/abnste/mewu_ai/releases/tag/v0.5.10)

完整的双语发行说明见 [docs/release-notes-v0.5.10.md](./docs/release-notes-v0.5.10.md)。 / See the consolidated bilingual notes in [docs/release-notes-v0.5.10.md](./docs/release-notes-v0.5.10.md).

## 未发布 / Unreleased

- **抓取 v4：OCR 误读链接自动纠错 + 清晰报错 + 未装 Scrapling 的内置基础抓取**：定位到此前"反复失败"的真正元凶——圈选识别用的本地 OCR 把微信文章 key 里的大写 `I` 认成小写 `l`（I/l/1、O/0 互混），30 次抓取里 19 次用了错误 URL；微信对无效 key 返回 HTTP 200 + "Parameter error" 错误页，与"封锁纯 HTTP"无关（实测有效 key 的纯 HTTP Fetcher 直接返回全文 1377 字）。v4 抓取脚本：微信 /s/ 链接返回错误页时**自动生成易混字符变体（全局 + 逐位替换，最多探测 24 个）并用轻量 Fetcher 探测**，命中即用纠正后的 URL 抓取并在结果/保存文件中注明；错误页不再当正文透传，而是返回结构化错误码（`weixin_error_page` / `network_unreachable`）并由界面翻译成明确提示（链接无效/需要代理/网络不可达）；非微信站点疑似被拦截（知乎 403）时升级隐身浏览器。**未安装 Scrapling 的用户**：新增内置基础抓取（`BasicHttpCrawlService`，纯 C# HttpClient + Chrome 头 + 剥标签抽正文，零依赖），静态页面直接出结果，动态页面（微信/知乎）失败后再引导一键安装；抓取子进程会剔除格式损坏的代理环境变量（如 `http://http://…`）但保留合法代理。实测：误读 URL `…clH…` 自动纠正为 `…IH…` 并抓到 1035 字正文；知乎回答（纯 HTTP 403）经隐身浏览器抓到全文；无效 key 返回清晰中文报错。/ **Crawl v4: auto-correcting OCR-misread links + clear errors + built-in basic fetch when Scrapling is absent**: the real culprit behind the repeated failures was the local OCR misreading uppercase `I` as lowercase `l` in WeChat article keys (19 of 30 attempts used a wrong URL); WeChat serves an HTTP 200 "Parameter error" stub only for invalid keys — a valid key returns the full article even over plain HTTP. v4 now generates bounded character-confusion variants (global + per-position, ≤24 probes) when a WeChat /s/ link returns the error stub, verifies them with the lightweight Fetcher, and crawls the corrected URL (noted in the result); error stubs are no longer relayed as content — structured error codes (`weixin_error_page` / `network_unreachable`) are translated into clear guidance (invalid link / proxy needed / unreachable); non-WeChat sites escalate to the stealth browser on block pages (Zhihu 403). Users without Scrapling get a new built-in basic fetch (`BasicHttpCrawlService`, pure C# HttpClient + Chrome headers + tag-stripping, zero dependencies) that handles static pages directly and falls back to the one-click install prompt for dynamic pages; malformed proxy env vars are stripped from the crawl subprocess while valid proxies are kept. Verified: the misread URL `…clH…` was auto-corrected to `…IH…` yielding the full 1035-char article; the Zhihu answer (plain HTTP 403) was fetched via the stealth browser; an invalid key returns a clear localized error.

- **修复微信文章抓取返回 "Parameter error" 噪声**（v3 stealthy-wechat）：日志与实测确认微信对纯 HTTP 请求（`Fetcher.get`，无 JS 渲染）返回 HTTP 200 + "Parameter error" 错误页，而此前的 passthrough 版本把该错误页全文直接透传给用户。现抓取脚本改为：**微信 URL 直接走 StealthyFetcher（隐身浏览器）**，从 `#js_content` 容器抽正文、`#activity-name` 取标题；非微信站点先快速 Fetcher、疑似错误页（含 "parameter error" 或正文 <80 字）再升级 StealthyFetcher。本机实测 `https://mp.weixin.qq.com/s/5svePaascIHkohiGhxZQiw` 抓到完整正文 1035 字 + 标题「金秋有约｜CURA阿迪达斯中国大学生路跑活动，等你来跑！」。/ **Fixed WeChat crawl returning "Parameter error" noise** (v3): WeChat serves an HTTP 200 "Parameter error" stub to plain HTTP fetches, which the passthrough build relayed verbatim. The fetch script now goes straight to StealthyFetcher for WeChat URLs (extraction via `#js_content` / `#activity-name`), and for other sites escalates from Fetcher to StealthyFetcher when the page looks like an error stub. Verified locally: full 1035-char body + title captured for the sample article.

- Scrapling 一键安装链路（v13）：圈选链接后点"安装 Scrapling 并爬取"会弹出对话框，mewuAI 自动探测 PATH 里的 Python 3.10+（`py` 启动器 / python3 / python），在 `%LOCALAPPDATA%\MewuAI\scrapling` 建 venv、pip install scrapling、scrapling install 下载隐身浏览器，实时上报进度（每一步都通过 PromptStatus 反馈）。完成后立即开始抓取，不需要再点一次。一键安装失败时显示错误并提供"复制手动安装步骤"按钮，把 `py -3.13 -m venv / pip install scrapling / scrapling install` 命令复制到剪贴板。未安装 Scrapling 时按钮显示为"安装 Scrapling 并爬取"，安装后变回"Scrapling 爬取"。/ Scrapling one-click install (v13): selecting a link and pressing "Install Scrapling & crawl" opens a dialog that detects Python 3.10+, creates a venv under `%LOCALAPPDATA%\MewuAI\scrapling`, runs pip install scrapling + scrapling install with live progress, then crawls automatically. A "Copy manual steps" button is offered on failure.

- 网易邮箱发件链路**改为强制 SMTP 授权码通道**：网页扫码拿到的 webmail 临时会话经实测会被服务端沙箱化（mbox:compose 返回 S_OK 但“已发送”箱查不到副本），因此扫码授权**仅用于读取邮件上下文**，不再代发邮件；`mewu-mail-send` 路由时若 SMTP 未配置则直接提示用户去 网页端 设置 → POP3/SMTP/IMAP 生成授权码后再发。/ NetEase Mail sending **now strictly requires the SMTP authorization code**: the web scan session is sandboxed by NetEase (S_OK without a Sent-folder copy), so QR authorization enables context-fetching, not sending. `mewu-mail-send` drafts without the SMTP code prompt the user to generate one in web Settings → POP3/SMTP/IMAP first.
- Scrapling 自动探测新增**快捷方式解析**（`ShellLinkResolver`）：不再只盯死 D:/scrapling_app 等已知路径，而是从桌面“爬虫”目录里扫所有 `.lnk` 快捷方式、解析其中的 UTF-16 LE 路径串，命中后直接定位到 scrapling 安装根目录（如 `D:\scrapling（爬虫）`），再检查 `venv\Scripts\python.exe` 是否存在。本机测试 `C:\Users\86152\Desktop\爬虫\scrapling启动桌面应用（快捷）.lnk` → `D:\scrapling（爬虫）\venv\Scripts\python.exe` 已存在，链路跑通。/ Scrapling now auto-detects via shortcut parsing: scans all `.lnk` files under the desktop “爬虫” folder, extracts the UTF-16 LE path inside, and returns the install root if `venv\Scripts\python.exe` exists. Verified locally: `C:\Users\86152\Desktop\爬虫\scrapling启动桌面应用（快捷）.lnk` → `D:\scrapling（爬虫）` (venv present).

- 修复网易邮箱网页扫码发件“UI 显示成功但实际未投递”误报：S_OK 后再查 fid=3 “已发送”箱，匹配主题+收件人+时间窗确认服务端真的写入了副本；若未在“已发送”箱找到副本，则返回明确的警告并强提示在 设置 → MCP → 网易邮箱 填写 SMTP 授权码通道获取可靠回执（不再误报“邮件已发送”）。/ NetEase web scan send no longer falsely reports "sent": after S_OK, the Sent folder (fid=3) is checked; if no copy is found, the warning explicitly tells the user to configure the SMTP authorization code for reliable delivery.
- 加固 Scrapling 抓取微信公众号：抓取脚本强制只取 #js_content 元素（不再 fallback 到 `get_all_text` 抓取页面 UI），增加启发式判别“看起来不像正文”（含 ≥2 个微信 UI 关键词、轻点赞/在看/分享/评论/Video/Mini Program/Parameter error 等、或文本 < 200 字符），自动探测路径增加桌面"爬虫"\scrapling、D:\爬虫\scrapling 等位置，让没装在 D 盘 scrapling_app 的用户也能直接用。/ Scrapling WeChat fetching tightened: only the #js_content container is now extracted, a UI-only heuristic discards residual UI text, and additional install paths are scanned (Desktop\爬虫\scrapling, D:\爬虫\scrapling).

- 网易邮箱新增网页版扫码授权（与 QQ 邮箱体验对齐）：点击“扫码授权”后本地渲染二维码（QRCoder，内容不经过第三方二维码服务），用手机“网易邮箱大师”App 扫码并确认，即取得网易网页登录会话（复刻 mail.163.com 登录页 mailscanlogin 协议：getqrcodeid → ngxqrcodeauthstatus 轮询 → qrcodeauth → ticketlogin），无需 IMAP/SMTP 授权码；会话（sid+Cookie）仅以 DPAPI 保存在本机。授权后 QQ 邮箱未授权时自动改用网易通道拉取收件箱实时上下文并代发邮件（网页接口 mbox:listMessages / mbox:compose），`mewu-mail-send` 草稿在未配置 SMTP 授权码时自动走网页发件；SMTP 授权码保留为备用发件通道。社区项目 netease-mail-mcp/NetEaseEmailConnector 因依赖 IMAP/SMTP 授权码、不支持纯二维码登录而未采纳。/ NetEase Mail now supports web QR-code authorization (matching the QQ Mail experience): “Scan QR code to authorize” renders the QR code locally (QRCoder; the payload never touches third-party QR services) and the user confirms with the NetEase Mailmaster mobile app, obtaining a webmail session by replicating the mail.163.com login page protocol (getqrcodeid → ngxqrcodeauthstatus polling → qrcodeauth → ticketlogin) — no IMAP/SMTP code needed; the session (sid+cookies) is stored locally with DPAPI only. When QQ Mail is not authorized the assistant now pulls live NetEase inbox context and sends confirmed mail through the web gateway (mbox:listMessages / mbox:compose), and `mewu-mail-send` drafts fall back to web sending when no SMTP code is configured; the SMTP code remains a fallback sending channel. The community projects netease-mail-mcp/NetEaseEmailConnector were evaluated but not adopted because they require IMAP/SMTP codes and offer no QR-only login.
- 修复 WorkBuddy 自动检测超时：桥接层从 stdio ACP（`--acp`，当前 CLI 版本在 session/new 上会因内部 HTTP 服务未启动而无限挂起）重写为 CLI 的本地 HTTP ACP 端点（`--serve` 模式 + `POST /api/v1/acp`，SSE/NDJSON 流），连接与模型读取实测约 3 秒完成；同时清洗继承的坏代理环境变量（如 `http://http://…`，此前会让 CLI 认证刷新永远失败）、记录 CLI stderr 诊断前缀、检测成功后固化 WorkBuddy.exe 路径。/ WorkBuddy auto-detection no longer times out: the bridge was rewritten from stdio ACP (whose `session/new` deadlocks in current CLI builds) to the CLI's local HTTP ACP endpoint (`--serve` + `POST /api/v1/acp`, SSE/NDJSON) — connection and model loading complete in ~3 seconds; inherited malformed proxy variables (e.g. `http://http://…`, which wedged the CLI's auth refresh) are now repaired, CLI stderr diagnostics are logged, and the discovered WorkBuddy.exe path is persisted.
- WorkBuddy 未配置不再阻塞其他设置保存：检测失败时，已配置过的沿用上次模型，从未配置的自动跳过 WorkBuddy 并以非阻断提示告知，邮箱、钉钉、飞书、Obsidian 等其余设置照常保存；检测失败时设置页仍提供兜底模型（上次保存值或 auto）供选择。/ A missing WorkBuddy configuration no longer blocks saving other settings: on detection failure previously-configured setups keep their saved model, never-configured ones skip WorkBuddy for that save with a non-blocking warning, while mail/DingTalk/Feishu/Obsidian settings save normally; the settings page also offers a fallback model (last saved value or auto) when detection fails.

- 修复卡顿：打开链接或邮件发送成功后自动收起识屏浮层（全屏冻结帧不再遮挡新窗口），AI 应答进行中则只收起工具栏不中断对话。/ Performance fix: after opening a recognized link or sending an email the capture overlay now dismisses itself (the frozen full-screen frame no longer covers the newly opened window); while an AI answer is in flight only the toolbar is hidden so the turn is not interrupted.
- 新增网易邮箱代发（MCP 栏目）：填入网易邮箱账号与 SMTP 授权码（网页端 设置 → POP3/SMTP/IMAP 生成，DPAPI 加密保存）后，识别到 163/126/yeah 等网易后缀地址即可直接撰写发送；QQ 邮箱未授权时也可用它给任意地址代发。AI 对话中的 `mewu-mail-send` 草稿会按收件人后缀自动选择 QQ 邮箱 MCP 或网易 SMTP 通道。/ New NetEase Mail sending (MCP tab): enter the mailbox account and SMTP authorization code (generated from web settings → POP3/SMTP/IMAP, stored with DPAPI) to email recognized 163/126/yeah addresses directly; any address can be reached when QQ Mail is not authorized. `mewu-mail-send` drafts from AI conversations auto-route between QQ Mail MCP and NetEase SMTP by recipient domain.
- 新增钉钉分享（MCP 栏目）：配置企业内部应用（AppKey/AppSecret/AgentId，Secret 加密保存）后，圈选截图的工具栏出现「钉钉」按钮，图片经工作通知发送给指定 userid 联系人。/ New DingTalk sharing (MCP tab): after configuring an enterprise app (AppKey/AppSecret/AgentId, secret encrypted locally), a DingTalk button appears on the selection toolbar and sends the captured image to the configured userids as a work notification.
- 新增飞书分享（MCP 栏目）：配置自建应用（AppId/AppSecret，需 im:message、im:chat 权限）后，圈选截图的工具栏出现「飞书」按钮，图片上传后发送到指定群或联系人；「测试连接」可拉取群列表直接挑选。/ New Feishu sharing (MCP tab): after configuring a self-built app (AppId/AppSecret with im:message and im:chat scopes), a Feishu button appears on the selection toolbar and uploads + sends the image to a chosen chat or contact; “Test connection” lists your chats for one-click selection.
- 新增 Obsidian 截图笔记（MCP 栏目）：自动检测本机 vault，圈选截图的工具栏出现「Obsidian」按钮，把图片存入附件目录并生成引用它的 Markdown 笔记（可选自动在 Obsidian 打开）。纯本地文件操作，无凭据。/ New Obsidian screenshot notes (MCP tab): local vaults are auto-detected; an Obsidian button appears on the selection toolbar, saving the image into the attachments folder and creating a Markdown note that embeds it (optionally opened in Obsidian). Fully local file operations, no credentials.

- 设置新增顶级「MCP」栏目（与「AI」「常规」等并列），QQ 邮箱从「AI 接入方式」移入该栏目。/ A new top-level “MCP” settings tab (alongside “AI”, “General” and others) now hosts QQ Mail, which moved out of the AI integrations tabs.
- 新增屏幕实体动作悬浮条：选区完成后自动提取其文本（窗口吸附选择走 UIA 无障碍文本，普通拖选走本地 OCR），识别到链接或邮箱时在选区旁弹出操作条——点击「打开链接」直接用默认浏览器打开；点击「发邮件给 …」弹出撰写窗口，经 QQ 邮箱 MCP 两阶段确认后发送（未授权则复制地址并打开网页邮箱，同时提示可扫码授权）。此前屏幕文本快照从未被填充导致 URL/邮箱识别失效的问题就此修复。/ New screen-entity action bar: right after a selection is made its text is extracted automatically (UIA accessibility text for window-snapped selections, local OCR for dragged regions); when a link or email address is recognized, a floating action bar appears next to the selection — “Open link” opens it in the default browser, and “Email …” opens a compose window that sends via QQ Mail MCP with two-phase confirmation (unauthorized users get the address copied and webmail opened, with a hint to authorize). This also fixes the long-standing issue that the screen text snapshot was never populated, which had disabled URL/email recognition.
- QQ 邮箱上下文注入改进：已授权发件但未识别到收件人地址时，明确告知模型可以代发、需先向用户询问收件人，避免模型臆断“当前环境不支持代发邮件”。/ Improved QQ Mail context injection: when sending is authorized but no recipient address was recognized, the model is now explicitly told it CAN send and should ask the user for the recipient, instead of hallucinating “sending is not supported in this environment”.
- 新增 QQ 邮箱 MCP 集成：接入官方 `api.mail.qq.com/mcp`（Streamable HTTP JSON-RPC），在设置的「AI → QQ 邮箱」页启用后，屏幕助手中出现邮件相关提问（含未读、收件箱、QQ 邮箱地址等意图）会自动拉取实时收件箱上下文注入提示词，任意对话渠道均可使用；拉取失败不影响正常对话。/ New QQ Mail MCP integration: connects to the official `api.mail.qq.com/mcp` (Streamable HTTP JSON-RPC). Once enabled on the AI → QQ Mail settings page, mail-related prompts in the screen assistant (unread, inbox, QQ addresses and similar intents) automatically fetch live inbox context into the prompt for any conversation channel; fetch failures never block the normal chat flow.
- QQ 邮箱改为独立扫码授权：每个用户在设置的「QQ 邮箱」页点击「扫码授权」，应用通过腾讯 OAuth 2.0（授权码 + PKCE，RFC 7591 动态注册）打开授权页，用手机 QQ 邮箱 App 扫码确认，即可以自己的账号获得独立令牌；令牌仅以 DPAPI 加密保存在本机，过期自动用 refresh token 刷新，不再读取或导入任何第三方客户端（含 WorkBuddy）的凭据。/ QQ Mail now uses its own QR-code authorization: each user clicks “Scan QR code to authorize” on the QQ Mail settings page; the app opens the Tencent authorization page via OAuth 2.0 (authorization code + PKCE, RFC 7591 dynamic registration) and the user confirms by scanning the QR code with the QQ Mail mobile app to obtain an independent token for their own account. Tokens are stored locally with DPAPI encryption, auto-refresh via refresh token, and no third-party client credentials (including WorkBuddy) are ever read or imported.
- 新增「识别邮箱即可代发」：提示词或屏幕/引用文本中识别到邮箱地址且用户明确要求发邮件时，模型输出 `mewu-mail-send` 草稿标记块，应用解析后遵循 QQ 邮箱 MCP 的两阶段确认协议——先展示发件人、收件人、主题与正文摘要，用户在对话框中点「确认发送」后才真正发送；收件人地址需通过实体校验，取消或失败均如实告知。/ Recognize-an-address-to-send: when an email address is recognized in the prompt or on-screen/referenced text and the user clearly asks to send, the model emits a `mewu-mail-send` draft block; the app parses it and follows the QQ Mail MCP two-phase confirmation protocol — showing from/to/subject/body first, sending only after the user clicks “Confirm send”. Recipient addresses must pass entity validation, and cancellations or failures are reported honestly.

## 0.5.9 — 录制交互重构与阴影修复 / Recording interaction redesign and shadow fixes

发行说明 / Release notes: [0.5.9](https://github.com/abnste/mewu_ai/releases/tag/v0.5.9)

- 录制期间不再安装全局鼠标钩子或重注入事件；遮罩和录制边框保持可见，覆盖层通过系统鼠标穿透把控制条之外的点击、滚轮和拖动直接交给底层应用，拖动经过控制条时也不中断。全屏无安全控制条时保留 F8 停止。 / Remove global mouse hooks and event re-injection during recording; keep the dimmer and recording border visible while system mouse transparency sends clicks, wheel input, and dragging outside the control strip directly to the underlying app, including drags crossing the strip. Full-screen captures with no safe control space retain F8 to stop.
- 停止录制继续使用有限超时恢复，避免停止或 Esc 后永久停在“处理中”。 / Keep a bounded stop timeout and restore path so stopping or pressing Esc cannot remain on “Processing” indefinitely.
- 控制条阴影使用无重复描边的独立图层；AI 标注文本卡为阴影预留独立透明边距，避免阴影裁切、白边和文字发虚。 / Render control-strip shadows on independent layers without duplicate borders, and give AI annotation cards dedicated transparent shadow padding to avoid clipping, white halos, and blurred text.
- 框选预览复用冻结截图，减少拖动期间重复创建裁剪位图的开销；最终截图保持原始像素。 / Selection previews reuse the frozen desktop image to avoid repeated bitmap crops while dragging; final captures retain their original pixels.

完整的双语发行说明见 [docs/release-notes-v0.5.9.md](./docs/release-notes-v0.5.9.md)。 / See the full bilingual notes in [docs/release-notes-v0.5.9.md](./docs/release-notes-v0.5.9.md).

## 0.5.8 — 全屏录制启动与输入稳定性 / Full-screen recording startup and input stability

发行说明 / Release notes: [0.5.8](https://github.com/abnste/mewu_ai/releases/tag/v0.5.8)

- 修复全屏录制倒计时结束后回退到普通截图工具条的问题；无安全空位的教学全屏采集隐藏控制条并保留 F8 完成/停止。 / Fix full-screen recording falling back to the screenshot toolbar after the countdown; teaching captures with no safe control space hide the bar and retain F8 to finish or stop.
- 录制输入转发不再吞掉和重注入每个鼠标移动事件，避免区外点击拖垮全局输入；普通录制即使低级钩子不可用也不会被误取消。 / Stop consuming and re-injecting every mouse move during recording so clicks outside the region cannot stall global input; ordinary recording no longer gets canceled when the low-level hook is unavailable.
- 录制窗口区域恢复增加无效 HWND 防护，避免录制结束时错误地关闭覆盖层。 / Guard region restoration against an invalid HWND so the overlay is not closed spuriously after recording.

完整的双语发行说明见 [docs/release-notes-v0.5.8.md](./docs/release-notes-v0.5.8.md)。 / See the full bilingual notes in [docs/release-notes-v0.5.8.md](./docs/release-notes-v0.5.8.md).

## 0.5.7 — 全屏录制输入与控件视觉修复 / Full-screen recording input and control visuals

发行说明 / Release notes: [0.5.7](https://github.com/abnste/mewu_ai/releases/tag/v0.5.7)

- 录制期间把控制条之外的鼠标移动、点击和滚轮输入转发到下层应用；控制条仍保持可暂停、继续和停止。 / While recording, forward mouse movement, clicks, and wheel input outside the control bar to the application underneath while keeping pause, resume, and stop controls interactive.
- 将录制、标注和截图工具条的阴影拆成独立图层，避免阴影裁切和文字模糊。 / Render recording, annotation, and capture-toolbar shadows in separate layers so shadows are not clipped and text stays sharp.
- 录屏倒计时、原位录制和结束后的窗口恢复继续通过 Release 录屏回放验证。 / Keep the countdown, in-place recording, and post-recording interaction restoration covered by the Release recording replay.

完整的双语发行说明见 [docs/release-notes-v0.5.7.md](./docs/release-notes-v0.5.7.md)。 / See the full bilingual notes in [docs/release-notes-v0.5.7.md](./docs/release-notes-v0.5.7.md).

## 0.5.3 — 流畅框选预览与录屏稳定性 / Smooth selection preview and recording stability

- 框选拖动期间复用冻结截图，减少重复裁剪造成的卡顿。 / Reuse the frozen desktop image while dragging to reduce selection-preview stutter.
- 改用更稳定的软件 H.264 编码路径，改善录屏播放的帧时间稳定性。 / Use the more stable software H.264 path to improve recording playback cadence.

## 0.5.2 — 多 API 协议、代理与历史会话 / Multi-API protocols, proxy and conversation history

发行说明 / Release notes: [0.5.2](https://github.com/abnste/mewu_ai/releases/tag/v0.5.2)

完整的双语发行说明见 [docs/release-notes-v0.5.2.md](./docs/release-notes-v0.5.2.md)。 / See the full bilingual notes in [docs/release-notes-v0.5.2.md](./docs/release-notes-v0.5.2.md).


## 0.5.1 — 公式、标注反馈与 README 演示 / Formula rendering, annotation feedback and README demos

发行说明 / Release notes: [0.5.1](https://github.com/abnste/mewu_ai/releases/tag/v0.5.1)

完整的双语发行说明见 [docs/release-notes-v0.5.1.md](./docs/release-notes-v0.5.1.md)。 / See the full bilingual notes in [docs/release-notes-v0.5.1.md](./docs/release-notes-v0.5.1.md).


- 历史 AI 回复使用与当前回复相同的 Markdown/公式渲染，重新截图加载历史后仍显示公式；取消历史回复的900字符显示截断，保留完整公式和复制内容。 / Render historical assistant replies with the same Markdown and math view as current replies, preserving formulas after reopening capture and removing the 900-character display truncation.

- 修复含二项式系数 `\binom` 的公式显示为源码的问题，覆盖流式最新回答及标注公式。 / Render binomial coefficients (`\binom`) in streamed replies and annotations.

- 标注气泡和原位文字标注支持公式排版及文字混排，长公式按框宽缩放，带标注导出保留相同公式效果。 / Typeset formulas alongside text in annotation cards and in-place labels, fit long formulas to their width, and preserve typesetting in annotated exports.

- 修复部分模型返回的公式因 `\_`、重复转义的 `\\,` 或常见希腊字母命令而显示成原始 LaTeX；现在会直接排版显示，同时复制仍保留原公式文本。 / Render formulas containing `\_`, double-escaped `\\,`, or common Greek-letter commands instead of exposing raw LaTeX, while preserving the original formula when copied.

- 最小化会话悬浮窗支持拖动：按住并移动可重新摆放，轻点仍恢复对话，关闭按钮保留独立点击行为。 / Minimized conversation widgets can be dragged to reposition; a click still restores the conversation and the close button keeps its own action.

- 最小化会话右侧显示旋转点环（悬停时由同位置的关闭按钮覆盖），回答成功完成后显示绿点；原“点击恢复对话”位置实时预览最新思考片段，没有思考内容时显示最新回答。取消与失败使用独立状态，多个会话分别更新。 / Show a rotating dotted indicator on the right of minimized conversations, covered by the close button on hover, and a green dot on successful completion. Preview the latest reasoning, or answer text when reasoning is unavailable, in place of “Click to restore”. Keep cancellation, failure and each conversation's state distinct.

- 扩大对话条拖动范围，顶部、底部及内容之间的空白均可拖动；输入框、按钮、滚动条与回答文字保留原有操作。 / Allow dragging from blank space at the top, bottom and between conversation content while preserving input, buttons, scrollbars and text selection.

- 对话条拖离底部后自动原位展开；浮动时可拖到包含任务栏的完整屏幕范围，拖回原位仍可吸附，取消拖动恢复原来的位置与展开状态。 / Automatically expand the conversation in place when detached from the bottom; floating conversations can reach the full screen including the taskbar area, retain docking, and restore their previous position and expansion state on cancellation.

- 扩大上拉箭头的透明点击区域，使其覆盖顶部拖动区而不改变现有视觉样式；拖动把手其余区域仍可正常拖动。 / Expand the history toggle's transparent hit area into the top drag zone without changing its visual appearance; the remaining drag handle stays draggable.

- 修复气泡对话仍分成历史和当前回复两块的问题：上拉菜单共用一个消息滚动区，实时回答完成后不重复显示，发送下一轮保留历史展开状态；移除没有实际回答时的“未收到 AI 回复”占位气泡。 / Use one message scroller for expanded conversations, keep the live answer only once after completion, preserve expanded history on subsequent sends, and remove fabricated empty-answer placeholder bubbles.

- 将上拉菜单中的对话统一为左右气泡：用户消息靠右、AI 消息靠左，当前回复使用同一套气泡表面；原位展开增加轻量位移动画，不再把历史和当前回复显示成两种卡片。 / Unify expanded conversations as left/right bubbles: user messages align right, AI messages align left, and the current reply uses the same bubble surface; in-place expansion now uses a light slide animation instead of two unrelated card styles.

- 修复历史对话悬停提示及模型/截图引用下拉层的阴影裁切：所有 Popup 现在为阴影预留透明边界，并在 100% / 175% / 200% 缩放及四个提示方向下通过实际渲染回放。 / Fix clipped shadows on the history hover tip and model / screenshot-reference popups: every Popup now reserves transparent shadow space and passes real render replays at 100%, 175%, and 200% scale in all four tooltip directions.

- 移除重复的独立对话窗口，拖动和展开历史始终保留原上拉菜单；右上角可最小化为悬浮按钮，恢复原冻结截图、标注和聊天，多个会话分别保留。气泡移除“你”和“AI”标签。 / Remove the separate chat window: dragging and expanding history retain the original conversation panel. Its top-right minimize button preserves the frozen screenshot, annotations and chat in independently restorable widgets. Remove speaker labels from bubbles.

- 重做教学批改为普通截图对话：移除旧的独立教学批改面板、页面收集器和侧栏核对入口。框选或上传内容后，用户可以直接在原有对话条用自然语言要求批改试卷、讲题、填答案、备课、解释材料或检查图形化编程 / Python；同一套提示覆盖语文、数学、英语及其他学科和非学科知识。公式按可复制 LaTeX 输出，代码保留语言标记和缩进，必要的短批注仍原位返回。 / Rework teaching review as ordinary screenshot conversation: remove the old teaching panel, page collector and side review entry. After selecting or uploading content, users can ask in the existing composer to grade papers, explain questions, fill answers, prepare lessons, explain materials, or review Scratch / Python code. The same guidance covers Chinese, mathematics, English and other subject or non-subject knowledge. Formulas use copyable LaTeX, code keeps language tags and indentation, and only necessary short annotations are placed in the original content.

- 将“新对话”移入对话条上拉菜单的标题区；收起时不再占用一行，展开后与对话记录标题和历史列表保持统一圆角浅色布局，并通过中英文、滚动和窄宽度回放验证。 / Move “New chat” into the expanded composer menu header. It no longer consumes a row when collapsed, keeps the same light rounded layout as the conversation history and list when expanded, and is verified in Chinese and English at normal, scrolled, and narrow widths.

- 对话条新增显式「新会话」边界（[PR #10](https://github.com/abnste/mewu_ai/pull/10)，shuziyuxingxing-stack）：一键开启空白会话，请求处理中会提示并拒绝；磁盘对话历史继续在历史面板展示，但不再注入新请求的上下文。补修：本机 Hermes 渠道同步重置服务端持久会话并清理当前渠道的应用内会话记忆，避免切换渠道后旧上下文回流；切换渠道时按新作用域重载历史展示。 / Add an explicit new-conversation boundary to the composer ([PR #10](https://github.com/abnste/mewu_ai/pull/10), shuziyuxingxing-stack): start a blank conversation in one click, with a clear refusal while a request is running; persisted history stays visible in the panel but is no longer injected into new request context. Follow-up: the local Hermes channel now resets its server-side persistent session and clears the in-app per-channel memory so stale context cannot return after switching channels, and switching channels reloads the history panel for the new scope.

## 0.5.0 — 可拖动对话条与截图交互 / Floating composer and capture interactions

发行说明 / Release notes: [0.5.0](https://github.com/abnste/mewu_ai/releases/tag/v0.5.0)

- 移除隐藏拖动横条残留的空行，恢复对话条原高度；透明拖动区复用现有顶部边距。 / Remove the empty drag-grip row and restore the composer height, reusing its existing top padding for dragging.

- 隐去对话条的可见拖动横条，保留顶部空白拖动区；拖动时显示原位虚线框，接近吸附范围时高亮，松手后消失。 / Hide the visible drag grip; show a non-interactive dashed docking target while dragging, highlighted within snap range.

- 对话条新增顶部拖动把手：越过拖动阈值后可在当前屏幕内固定摆放，不再自动收纳；拖回底部原位附近松手，以弹性动画吸附并恢复自动隐藏。 / Drag the composer handle past the detachment threshold to keep it visible at a floating position; release near its original bottom dock to spring back and restore auto-hide.

- 修复更新接口限流时误请求不存在的校验文件而返回 404；无法获取可信校验值时明确提示重试，不绕过校验。框选后的输入焦点保护现在会在鼠标主动移动后解除，恢复选区与工具条悬浮收纳。 / Stop requesting invented checksum assets after GitHub rate limits; retain verification requirements and explain retry options. Release post-selection typing protection on deliberate pointer movement so selection and toolbar hover hide the composer again.

- 修复新截图被旧贴图遮挡：旧贴图保持在本轮截图下方，本轮新贴图仍显示在上方；改善文字标注暂时失焦后的连续编辑与样式修改。 / Keep existing pins below a new capture while allowing newly created pins above it; preserve annotation editing and style selection across temporary window deactivation.

- 基于 [PR #10 第【2】项](https://github.com/abnste/mewu_ai/pull/10) 中 shuziyuxingxing-stack 的 JSON 防护贡献，接入响应字段与类型校验；拒绝畸形流式终态，规范非流式错误，并修复本机代理缺字段及畸形 RPC 错误包的异常收尾。 / Integrate shuziyuxingxing-stack’s JSON guards from PR #10 item 2: validate response fields and types, reject malformed stream termination, report invalid non-streaming responses, and safely handle missing fields and malformed local-agent RPC errors.

- 采纳并补修 [PR #10](https://github.com/abnste/mewu_ai/pull/10)：Codex / WorkBuddy 支持手动选择本机可执行文件，设置窗口支持最小化；修复所选路径保存及环境 Provider 导入时的路径保留；Codex 可沿用本机 ChatGPT 或 API Key / CCSwitch 登录配置，不再强制覆盖为官方 OpenAI Provider。 / Adopt and complete [PR #10](https://github.com/abnste/mewu_ai/pull/10): allow manually selecting local Codex / WorkBuddy executables and minimizing the settings window; persist selected paths across saves and environment-provider imports; and let Codex inherit local ChatGPT or API-key / CCSwitch authentication instead of forcing the official OpenAI provider.

## 0.4.8 — Issue #9 API 地址与 DeepSeek 状态 / Issue #9 endpoint recovery and DeepSeek states

- 修复 [Issue #9](https://github.com/abnste/mewu_ai/issues/9) 中 API 模型目录在裸地址返回网页或错误 JSON 时设置页可能崩溃的问题。现在有限尝试原地址、`/v1` 和 `/api/v1`；成功后把可用地址显示为未保存草稿，保存后用于后续请求，全部失败才提示 API 错误。 / Fix [Issue #9](https://github.com/abnste/mewu_ai/issues/9): recover model catalogs when a bare endpoint returns HTML or invalid JSON. Try the original address, `/v1`, and `/api/v1` within a bounded sequence; show a successful endpoint as an unsaved draft for later requests, and report an API error only after all candidates fail.
- 改进 [Issue #9](https://github.com/abnste/mewu_ai/issues/9) 中 DeepSeek 思考流的状态反馈：收到 `reasoning_content` 增量时显示仍在思考，只有终态没有正文时才显示思考-only 错误；不关闭思考、不降低输出预算。 / Improve [Issue #9](https://github.com/abnste/mewu_ai/issues/9) DeepSeek thinking-stream feedback: show that reasoning is still in progress when `reasoning_content` arrives, and report reasoning-only failure only after a terminal response without an answer; do not disable thinking or reduce output budgets.
- 部分采纳 [PR #8](https://github.com/abnste/mewu_ai/pull/8) 的 DeepSeek 思考开关适配与回归测试。 / Partially adopt the DeepSeek thinking-toggle compatibility and regression coverage from [PR #8](https://github.com/abnste/mewu_ai/pull/8).

## 0.4.7 — API 回复与设置稳定性 / API replies and settings stability

- 修正视觉问答中完整代码块回答被误判为损坏 JSON、继而提示“只有思考”的问题；保留正常回答，继续拒绝真正截断的回复。 / Preserve complete fenced-code answers in visual conversations instead of mistaking them for broken JSON and reporting reasoning only. Genuinely interrupted responses are still rejected.
- API 设置在填写地址时先校验未完成输入，格式错误的密钥显示提示而不使自动模型加载崩溃；设置窗口增加最大化/还原按钮，支持双击标题栏切换，默认仍保持紧凑尺寸。 / Validate endpoint drafts before loading credentials and report malformed API keys without crashing automatic model loading. Add maximize/restore buttons and title-bar double-click support while keeping the compact default window size.

- 补齐兼容 API 的分块与对象形式正文读取，严格区分正文、思考和未知内容类型；批注 JSON 尾部损坏时保留完整有效回答与已有标注，不将截断的网络回复当作完成。 / Read supported typed and object-form answers from compatible APIs while keeping reasoning and unknown content out of the answer. Preserve a complete valid answer and existing annotations when annotation JSON is malformed, without treating interrupted responses as complete.

- [完整说明 / Full notes](./docs/release-notes-v0.4.7.md)

## 0.4.6 — API 接入、标注编辑与应用快照 / API connections, annotation editing and app snapshots

- 按 2026 年 9 月官方接口更新国内外 API 接入模板，添加服务商分组与搜索；完善模型目录格式、分页及对话模型筛选，适配新推理模型参数、图文能力和类型化流式回复。已有连接、密钥及默认选择保持原样。 / Update China and global API presets against September 2026 documentation, with grouped service search, model catalog pagination and chat filtering, current reasoning-model parameters, vision capabilities and typed streaming responses. Existing connections, keys and defaults are preserved.

- 颜色选择改为可直接点选、拖动的全彩色板：外圈选色相，中间调整饱和度与明暗，保留同步的 RGB 和 HEX 精确输入。 / Replace RGB sliders with a full-color palette: choose a hue on the ring and saturation and brightness on the center plane, with synchronized RGB and HEX inputs.

- 整理手工标注工具分组并补充可调整两端点的直线。按 Shift 绘制或调整时，直线和箭头锁定水平、垂直或 45°，矩形保持正方形，椭圆保持正圆。序号圆底更紧凑，删除后优先复用空缺编号；选中对象可改色，文字字体、字号和荧光底色可修改并撤销重做。空白文本草稿离开后自动清理，马赛克拖动时直接预览真实像素化效果。 / Group manual annotation tools and add lines with editable endpoints. Hold Shift while drawing or resizing to constrain lines and arrows to horizontal, vertical or 45°, rectangles to squares, and ellipses to circles. Number markers are more compact and reuse deleted numbers. Selected objects support color changes; text fonts, sizes and highlights support undo and redo. Empty text drafts are removed when abandoned, and mosaic drags preview real pixelation.

- 恢复紧凑的设置窗口高度，修正 API 连接默认标签和更多操作图标居中、卡片底部留白、刷新图标与菜单阴影裁切。手工标注仅在点击“完成”后自动复制，绘制、编辑和撤销重做期间保留原剪贴板；AI 标注返回后仍可自动复制。复制图不包含编辑光标、焦点框或控制点。 / Restore compact settings height and fix default-badge and menu-icon alignment, card padding, the refresh icon and clipped menu shadows. Manual annotations copy only when Finish is clicked; drawing, editing, undo and redo preserve the clipboard. Returned AI annotations still copy automatically. Copies exclude editor carets, focus borders and handles.

- API 设置改为可命名的连接列表，点击原地展开，添加时选择服务商；提供重命名、设为默认和删除操作。切换时保留模型、密钥及未完成的高级设置草稿，编辑或测试备用连接不再更改默认连接。 / API settings now show named connections with inline editors and service selection when adding. Rename, set a default, or remove connections from their menus. Switching preserves model, key and unfinished advanced-setting drafts; editing or testing a backup connection no longer changes the default.

- 修正设置页复选框勾号偏移，增加 API 接入点操作按钮与列表的间距，并统一 API Key 输入框及相邻按钮的高度。 / Center settings checkmarks, separate endpoint actions from the selector, and align the API key field and its adjacent button with the other form controls.

- 截图画布支持右键穿透：右键点击或按住拖动操作底层应用的左键，Ctrl＋右键传递真正的右键。松开后恢复截图界面并保留已有截图；对话条、工具栏和 OCR 的右键菜单保留。 / On the capture canvas, right-click or hold and drag to operate the underlying application's left button; Ctrl + right-click forwards a real right-click. Releasing restores the overlay and preserves existing captures. Composer, toolbar and OCR context menus remain available.

- 无延时截图在快捷键所在的界面线程直接冻结画面，去掉启动过程的两次额外排队，避免抓取时机落到后续画面。 / Zero-delay screenshot requests freeze the current frame directly on the hotkey UI thread, eliminating two unnecessary dispatcher hops.

- 修复置顶长图放大时受窗口尺寸约束而变形的问题，窗口和图片同步等比例缩放；改进滚动拼接接缝，避免重复保留上一帧的底部边框。 / Fix pinned long images distorting at native window size limits, and join scrolling frames within their overlap to avoid repeating bottom borders.
- 自动吸附窗口后可截取应用快照：在前台保持冻结画面，通过系统窗口渲染接口取得可访问滚动区域的原始图像并自动拼接，完成后恢复滚动位置、置顶并引用；取消或关闭也先恢复位置。依赖应用提供可用的渲染和滚动接口，不以文字重排代替原图，不将拼接失败当作完整截图。手动拖选仍保留滚动长截图。 / Window-snapped selections can capture original pixels from an accessible scroll area through Windows compositor capture while the foreground stays frozen. Capture restores the original scroll position, then pins and references the result; cancellation and closing also restore first. Rendering and scroll support depend on the application. No reflowed text substitution or incomplete captures reported as complete; manual selections retain scrolling capture.
- 长截图取消固定 24 段限制，改用增量拼接，仅保留合成图与最新匹配帧，按实际图像容量控制内存。 / Remove the fixed 24-segment limit and merge incrementally, retaining the composite and latest matching frame within the image capacity budget.

- [完整说明 / Full notes](./docs/release-notes-v0.4.6.md)

## 0.4.5 — 回复解析与贴图层级 / Reply parsing and pinned windows

- 修复刚置顶的贴图被当前截图层遮挡；贴图保持在上方，之后开启的截图层位于已有贴图下方。 / Keep newly pinned images above the current capture overlay and later capture overlays below existing pins.
- 双语 README 增加数据标注和表格识别实图，扩展为八宫格。 / Expand the bilingual gallery to eight examples with data annotation and table recognition screenshots.
- 修复截图问答中模型前言和未转义引号导致整段 JSON 显示的问题；保留可恢复的正文、分段及通过校验的批注。 / Fix raw JSON appearing in screenshot answers when a model adds a preamble or unescaped quotes; retain recoverable text, paragraphs and validated annotations.

- 更新中英文独立封面，并补充 Windows 平台说明和合作项目入口。 / Add separate localized covers, clarify Windows support and link to a partner project.
- [完整说明 / Full notes](./docs/release-notes-v0.4.5.md)

## 0.4.4 — 原位翻译与窗口交互 / In-place translation and window interaction

- 翻译批次并行处理，改善原文与译文的定位及复制、贴图、导出一致性。 / Process translation batches concurrently and improve placement across the overlay, copies, pinned images and exports.
- 修复贴图后的截图层级，调整主界面防捕获以减少录屏冲突。 / Fix capture stacking over pinned images and adjust main-window capture protection.
- 双语 README 改为六宫格演示，并加强源码与发行包隐私检查。 / Add a six-cell bilingual preview gallery and strengthen repository and package privacy checks.
- [完整说明 / Full notes](./docs/release-notes-v0.4.4.md)

## 0.4.3 — 表格识别与回复容量 / Table recognition and response capacity

- MiniMax-M3 问答使用官方最大输出并保留思考，移除固定 8192 token 限制。 / Use M3's documented maximum output while retaining thinking, removing the fixed 8192-token limit.
- 精简表格请求，保留原批注，完整回复后呈现表格；翻译超限和长行自动拆分补译。 / Simplify table requests, preserve annotations and display completed tables; recover translation limit failures with smaller sections.
- 移除独立试卷入口，沿用普通截图问答；更新双语说明。 / Remove the separate paper workflow entry and use ordinary screenshot conversations.
- [完整说明 / Full notes](./docs/release-notes-v0.4.3.md)

## 0.4.2 — 原卷批注与数学排版 / On-paper feedback and math typesetting

- 错因与正确公式在原卷旁显示、避让作答和密集文字，并随带标注试卷导出。 / Place and export feedback beside answers while avoiding other handwriting and dense text.
- 新增整页查看、公式预览与原文编辑；AI 回复支持常用 LaTeX 公式，复制保留原文。 / Add full-page viewing, formula previews, source editing and copyable LaTeX in replies.
- 修正英文演示知识点，强化批改内容语言要求；随包添加数学组件及字体许可。 / Correct English knowledge points, clarify grading output language and package math/font licenses.

## 0.4.1 — 多步计算与批改说明 / Calculation steps and review explanations

- 修复计算步骤挤在同一行，保留换行并支持回车、自动折行和滚动。 / Preserve calculation step breaks with multiline editing, wrapping and scrolling.
- 批改说明可编辑、确认并随逐题 CSV 导出，避免改判后仍显示旧说明。 / Edit and export explanations together with reviewed verdicts.
- README 展示官方真实手写答卷经逐题核对后的完整流程。 / Show the reviewed workflow on an official handwritten exam script.

## 0.4.0 — 教学批改与多页作业 / Reviewed grading and multi-page assignments

- 新增 PDF 选页导入、跨次截图收集、身份去重、双阶段逐题批改与老师核对。/ Added selected PDF imports, cross-capture collections, identity checks, two-stage grading and teacher review.
- 支持共同错题统计、练习编辑及标注页/表格/分离答案的教学包导出。/ Added shared-error summaries, editable practice and teaching-pack export.
- PDF 渲染使用独立有界进程，修复本机 AMD 驱动退出异常。/ Isolated PDF rendering to avoid the reproduced AMD shutdown crash.
- 修复试卷批改中同数字题干抢占答案批注位置，保留数学正负号并限制 OCR 校准范围。/ Prevented repeated text in exam questions from pulling annotations away from student answers.
- 保留回复列表中的原始题号，修正显示与复制从 1 重新编号的问题。/ Preserved original question numbers in displayed and copied lists.
- 补充教学任务的证据、评分与出题规则，明确未生成批注的状态。/ Added evidence and scoring guidance for teaching tasks and clear feedback when annotations are missing.
- 双卷最终实测 8 项符合参考、4 项进入待核；手写识读仍须核对，视觉渠道不可用时明确失败。/ The final live two-page replay yielded eight matching states and four uncertain states; handwriting and provider availability still require review.

## 0.3.4 — 对话图片与翻译排版 / Reply images and translation layout

- 修复 AI 回复图片不显示，接入 Hermes 本地图片、MEDIA 引用及图片工具结果。/ Fixed missing reply images, including local images, MEDIA references, and image-tool results from Hermes.
- 去掉图片的灰色底板和程序标签，复制时保留说明、文字与表情。/ Removed gray image backdrops and app-added labels while preserving descriptions, text, and emoji when copying.
- 修复长译文及双栏译文重叠，统一显示、选择和导出布局。/ Fixed overlapping translations across lines and columns, with consistent display, selection, and export layouts.

[完整双语说明 / Full notes](./docs/release-notes-v0.3.4.md) · [Release](https://github.com/abnste/mewu_ai/releases/tag/v0.3.4) · [v0.3.3 → v0.3.4](https://github.com/abnste/mewu_ai/compare/v0.3.3...v0.3.4)

## 0.3.3 — 视频置顶、思考光效与开源许可 / Pinned videos, activity glow, and license notices

- 修复带标注视频置顶后的闪退。/ Fixed crashes after pinning annotated videos.
- 新增可开关、可调 RGB 颜色的底部思考光效，修复任务栏区域截断。/ Added a configurable AI activity glow that reaches the full display edge.
- 统一历史与最新回复的复制菜单，修复阴影裁切，保留完整内容和表情。/ Unified reply copy menus, fixed clipped shadows, and preserved full text and emoji.
- 关于页新增第三方组件与完整许可的离线查看入口。/ Added offline access to third-party components and full license notices in About.

[完整双语说明 / Full notes](./docs/release-notes-v0.3.3.md) · [Release](https://github.com/abnste/mewu_ai/releases/tag/v0.3.3) · [v0.3.2 → v0.3.3](https://github.com/abnste/mewu_ai/compare/v0.3.2...v0.3.3)

## 0.3.2 — 视频时间、对话输入与中文显示 / Video timing, input, and Chinese text

- 修复 MiniMax 视频时间压缩为零点几秒，以及末尾帧不完整引发的 HTTP 400；保留原视频。/ Fixed compressed MiniMax timestamps and HTTP 400 errors caused by incomplete final frame groups, preserving the original video.
- 修复视频播放/暂停状态、对话条弹出焦点、录制后直接输入和 Ctrl+C 复制回答。/ Fixed playback controls, input focus when the bar appears, typing after recording, and Ctrl+C copying.
- 统一中文回复字体，默认简体中文，保留明确的语言要求、引用与代码。/ Unified Chinese reply fonts and defaulted Chinese answers to Simplified Chinese while preserving explicit language requests, quotations, and code.
- 新增跨截图关联箭头，改善多选区工具条显隐与逐行翻译。/ Added cross-screenshot connection arrows and improved selection toolbars and line-by-line translation.

- 修复首轮回答显示后继续提问仍长时间停在“AI 正在分析”的问题；后台补标会被安全取消并让出请求。/ Fixed follow-up questions remaining stuck on “AI is analyzing” after the first answer; background annotation repair now yields to the new request safely.

- 修复多区域提问期间工具条只响应一个选区的问题，等待回答和后台补标时也能悬停切换。/ Fixed toolbars responding to only one region during multi-region questions, including while waiting for an answer or additional annotations.

- 截图框选、移动或缩放完成后自动聚焦对话输入，鼠标悬停不再打断刚开始的输入。/ The conversation input receives focus after selecting, moving or resizing a screenshot, and hovering no longer interrupts typing.

- 教学演示默认开启，移除截图左上角的共享状态标识；仍可在设置中关闭。/ Teaching mode is on by default, without a sharing-status badge on the capture overlay. It can still be turned off in Settings.

- 教学演示模式支持录屏和长截图，控制条与预览自动避开采集区。全屏没有空位时可用 F8 停止/完成，倒计时中可取消。/ Teaching mode now supports recording and scrolling capture. Controls and previews stay outside the capture area; F8 stops or finishes capture and cancels the countdown.

[完整双语说明 / Full notes](./docs/release-notes-v0.3.2.md) · [Release](https://github.com/abnste/mewu_ai/releases/tag/v0.3.2) · [v0.3.1 → v0.3.2](https://github.com/abnste/mewu_ai/compare/v0.3.1...v0.3.2)

## 0.3.1 — 视频请求诊断 / Video request diagnostics

- 视频请求失败时显示实际渠道、可识别的失败原因、服务代码和安全的追踪编号，不再只显示笼统的 HTTP 状态。
- 对 MiniMax 返回的“视频内容被拒绝”也给出明确提示，避免误导用户反复压缩一个本身未超限的文件。
- 错误处理不会回显服务端可能包含的提示词、媒体数据或认证信息。
- 统一 API 与 MiniMax Code 的非成功响应处理，并补齐视频 422 的安全回归测试。
- 原位翻译可识别常见的 JSON 包装和空白 OCR 行；未按原行数返回时会自动重试一次，再给出可操作的提示。

[完整双语说明 / Full notes](./docs/release-notes-v0.3.1.md) · [Release](https://github.com/abnste/mewu_ai/releases/tag/v0.3.1) · [v0.3.0 → v0.3.1](https://github.com/abnste/mewu_ai/compare/v0.3.0...v0.3.1)

## 0.3.0 — 多渠道 AI 与模型切换 / Multiple AI channels

- 新增 ChatGPT Work / Codex、WorkBuddy、MiniMax Code 桌面接入；保留 API、Hermes。
- 多 API 接入列表、配置共存、单层模型菜单、记住上次渠道和模型。
- 修复纯文字视觉提示、原始 JSON 回答、WorkBuddy 思考/连接等待与模型选项兼容。
- 统一 AI 设置、主页渠道状态、快捷键清空和模型弹层布局。
- 正式采用 MPL-2.0，补齐社区规范；README 使用 GIF，下载资产仅安装 EXE 与便携 ZIP。

[完整双语说明 / Full notes](./docs/release-notes-v0.3.0.md) · [Release](https://github.com/abnste/mewu_ai/releases/tag/v0.3.0) · [v0.2.6 → v0.3.0](https://github.com/abnste/mewu_ai/compare/v0.2.6...v0.3.0)

## 历次正式发布 / Previous published releases

以下版本已在 GitHub 发布安装包。原发行说明通过版本标签永久定位，仓库内文档保留原文。/ These versions have published packages; tagged notes preserve the original release text.

| 版本 / Version | 主要内容 / Highlights | 原发行说明 / Original notes |
| --- | --- | --- |
| [0.2.6](https://github.com/abnste/mewu_ai/releases/tag/v0.2.6) | 标注对象直接编辑、RGB 复制、录屏电脑/麦克风音频、MP3 导出、Hermes 安装后启动、历史复制和高 DPI 菜单 / Annotation handles, RGB copying, recording audio, MP3 export, Hermes startup, history and DPI fixes | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.2.6/docs/release-notes-v0.2.6.md) |
| [0.2.5](https://github.com/abnste/mewu_ai/releases/tag/v0.2.5) | 教学共享、增量回答与交互性能、滚动箭头、设置自动检查更新、录屏测试时序修复 / Teaching mode, streaming performance, scroll arrows, automatic update checks, recording test timing | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.2.5/docs/release-notes-v0.2.5.md) |
| [0.2.3](https://github.com/abnste/mewu_ai/releases/tag/v0.2.3) | Hermes 启动兼容、API 提供商配置、请求参数和贴图阴影 / Hermes startup, API configuration, request parameters and pinned-image shadows | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.2.3/docs/release-notes-v0.2.3.md) |
| [0.2.2](https://github.com/abnste/mewu_ai/releases/tag/v0.2.2) | 自包含包还原与慢机器录屏取消等待，包含 0.2.0 / 0.2.1 的功能 / Self-contained restore and recording cancellation, including 0.2.0 / 0.2.1 changes | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.2.2/docs/release-notes-v0.2.2.md) |
| [0.1.0](https://github.com/abnste/mewu_ai/releases/tag/v0.1.0) | 稳定性与隐私、马赛克坐标、结构化回答、OCR/翻译、历史抽屉 / Stability, privacy, mosaic coordinates, structured answers, OCR/translation and history drawer | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.1.0/docs/release-notes-v0.1.0.md) |
| [0.0.11](https://github.com/abnste/mewu_ai/releases/tag/v0.0.11) | 关于页与设置导航裁切 / About page and settings navigation clipping | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.0.11/docs/release-notes-v0.0.11.md) |
| [0.0.10](https://github.com/abnste/mewu_ai/releases/tag/v0.0.10) | 上传按钮、流式布局、Hermes 状态、批注、翻译与长截图 / Upload button, streaming layout, Hermes status, annotations, translation and scrolling capture | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.0.10/docs/release-notes-v0.0.10.md) |
| [0.0.9](https://github.com/abnste/mewu_ai/releases/tag/v0.0.9) | Release 资产补传 / Release asset upload recovery | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.0.9/docs/release-notes-v0.0.9.md) |
| [0.0.8](https://github.com/abnste/mewu_ai/releases/tag/v0.0.8) | 安装器版本同步、思考与标注布局、附件引用 / Installer version sync, reasoning and annotation layout, attachment references | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.0.8/docs/release-notes-v0.0.8.md) |
| [0.0.4](https://github.com/abnste/mewu_ai/releases/tag/v0.0.4) | 智能框选、长截图、马赛克与统一图像/视频批注 / Window selection, scrolling capture, pixelation and image/video annotations | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.0.4/docs/release-notes-v0.0.4.md) |
| [0.0.1](https://github.com/abnste/mewu_ai/releases/tag/v0.0.1) | 首个 Windows x64 公开版本 / First public Windows x64 release | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.0.1/docs/release-notes-v0.0.1.md) |

## 保留的发布准备记录 / Retained release preparation records

以下文档和标签属于准备或失败构建，没有对应的正式 Release 下载；不能把它们当成已成功发布。/ The following preparation or failed-build records have no corresponding published Release downloads.

- [0.2.4](./docs/release-notes-v0.2.4.md)：正式构建被 GIF 录屏测试拦截，改由 0.2.5 发布。
- [0.2.1](./docs/release-notes-v0.2.1.md)、[0.2.0](./docs/release-notes-v0.2.0.md)：还原/录屏测试未完成发布，功能进入 0.2.2。
- [0.0.7](./docs/release-notes-v0.0.7.md)、[0.0.3](./docs/release-notes-v0.0.3.md)、[0.0.2](./docs/release-notes-v0.0.2.md)：保留的早期发布准备文档。

## 2026-09-07 文档与附件修正 / Documentation and asset correction

此前把 v0.2.6 之后的文档清理追加到 v0.2.6 Release 是一次发布记录错误，现已恢复原始功能说明，新功能按 v0.3.0 独立归档；旧标签和安装包未移动或替换。v0.2.6 的 SHA256SUMS.txt 在 2026-09-07 被移除，而不是发布时没有提供。用户从 v0.2.5 及更早版本升级时应手动下载安装器，避免旧更新器找不到校验文件。

An earlier edit incorrectly added later documentation cleanup to the v0.2.6 release. Its original feature notes are restored, and subsequent features are recorded under v0.3.0. Existing tags and binaries are unchanged. The v0.2.6 checksum attachment was removed on 2026-09-07; it was present at the original release. Users of v0.2.5 or earlier should download the installer manually because those updaters require the old checksum file.
