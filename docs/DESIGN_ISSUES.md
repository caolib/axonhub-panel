# 设计 / 正确性问题审查记录（DESIGN_ISSUES）

> 本文档仅记录代码审查中发现的**设计 / 正确性**问题，**未修改任何源代码**。
> 整理自一次人工代码审查，按严重程度分组：严重（Serious）/ 中等（Medium）/ 轻微（Minor）。

## 修复进度汇总

> 更新时间：2026-09-19（第二轮：修复轻微项 #9–#16）。本轮已实际运行 `cargo check --tests`（0 警告）与 `cargo test`（**68 passed / 0 failed**，较上轮新增 2 条针对性用例），整机编译与单测全绿；静态复核与凭据落日志核查均已执行。
> 状态图例：✅ 已修复 / 🔄 进行中 / ⬜ 未处理（超出本轮范围）。

| 编号 | 问题一句话 | 状态 | 落地文件 | 验证结果 |
| --- | --- | --- | --- | --- |
| #1 | ureq 未启用 TLS 后端，HTTPS 连不上 | ✅ 已修复 | `Cargo.toml:10` | `Cargo.lock` 含 `rustls`(464) 与 `rustls-platform-verifier`(498)，被 ureq 引用(780) |
| #2 | Worker 持有冻结 Config，UI 改的 endpoint/project_id 传不到轮询线程 | ✅ 已修复 | `worker.rs` / `app.rs` / `main.rs` | `Command::SetEndpoint`(57)/`SetProject`(59)/`apply_command`(114)；`main.rs:1928`、`app.rs:123` 已下发；`Pause` 已删除 |
| #3 | 配置损坏静默回退默认值，丢设置无提示 | ✅ 已修复 | `config.rs` | `ConfigLoadStatus`(187)/`load_with_status`(195)/`preserve_corrupt`(281)/`try_save`(239)；`load`/`save` 签名不变(233/251) |
| #4 | 密码无法输入/粘贴非 ASCII，中文密码被静默丢弃 | ✅ 已修复 | `src/ui/login.rs` | `accepts()`(162) 新增，`insert`(169)/`paste`(185) 共用 |
| #5 | 整条「暂停」功能是无用死代码 | ✅ 已修复 | `worker.rs` / `app.rs` / `main.rs` | 三处均无 `Pause`/`paused`/`MENU_PAUSE`；`MENU_SIGNIN` 仍在 `main.rs:229`（未误删） |
| #6 | SignedIn 解析的 project_id 不写盘，重启回退 | ✅ 已修复 | `app.rs` | `SignedIn` 分支含 `self.save()`(121) |
| #7 | 全程缺日志/追踪，后台失败只能看状态栏 | ✅ 已修复 | `Cargo.toml` / `main.rs` / `config.rs` / `client.rs` / `worker.rs` / `app.rs` | 引入 `tracing` + 文件 subscriber（`main.rs:503-513`）；吞错点插桩（`config.rs`/`client.rs`/`worker.rs`/`app.rs`）。`cargo check` + `cargo test` 全绿（66 passed/0 failed）；日志经复核不写凭据 |
| #8 | 退出被阻塞请求拖住最多 12 秒 | ✅ 已修复 | `worker.rs` | 主循环改用 `recv_timeout` 分段等待(302)，取代 `std::thread::sleep` |
| 模型名括号 | 显示模型名用 `「」` 包裹 | ✅ 已修复 | `model.rs` | `display_model()`(382-387) 改为 `[]` 包裹 |
| #9 | main.rs 上帝模块 | 🔄 部分处理 | `src/ui/clipboard.rs` / `main.rs` | 剪贴板读写抽出为 `ui::clipboard` 模块，`main.rs` 2762→2710 行；完整窗口过程拆分仍留待后续（见下） |
| #10 | 两个窗口过程重复 Win32 样板 | ✅ 已修复 | `main.rs` | 抽出 `with_double_buffer()`(1494)，`paint`(1524)/`paint_detail`(2664) 共用；工作区约束早已共用 `clamp_window_rect` |
| #11 | token.rs base64url 对 `+`/`/` 过宽容忍 | ✅ 已修复 | `src/token.rs` | `value()`(58-61) 仅接受 `-`/`_`；新增用例 `rejects_classic_base64_alphabet` |
| #12 | model.rs 计数基于被截断的 executions | ✅ 已修复 | `src/model.rs` | 新增 `attempts_truncated` 字段；`is_error()`(375-383) 截断时保守返回 true；新增用例覆盖 12 次尝试场景 |
| #13 | app.rs 过滤在 truncate 之后执行 | ✅ 已满足 | `src/app.rs` | 代码实为「先 `retain` 过滤(180)再 `truncate`(183)」，原描述滞后于代码，无需改动 |
| #14 | 双缓冲绘制对 GDI 空句柄无防护 | ✅ 已修复 | `main.rs` | `with_double_buffer()`(1500-1507) 对 `CreateCompatibleDC`/`CreateCompatibleBitmap` 做 `is_invalid()` 检查，失败即释放并返回 |
| #15 | theme.rs 裸 `unsafe impl Send for Fonts` | ✅ 已修复 | `src/theme.rs` | 移除 `unsafe impl Send`，改为 `PhantomData<Rc<()>>`(95-99) 使 `Fonts` 成为 `!Send`/`!Sync`，以类型系统表达「仅 UI 线程」 |
| #16 | paint/save 等写入错误被静默忽略 | ✅ 已修复 | `config.rs` / `main.rs` / `src/ui/clipboard.rs` | `config.save()` 已 `warn!`；剪贴板 `set_clipboard_text` 三处失败分支均 `warn!`（打开/分配/写入） |

---

## 仓库概览

- **语言 / 技术栈**：Rust + Win32 API + GDI+。
- **UI 实现**：纯手写窗口过程（`WindowProc`），**非 egui / 非 webview**，使用 Win32 原生窗口与 GDI+ 双缓冲绘制。
- **构建配置**：`panic = "abort"`（无 unwind 栈展开，崩溃即进程退出）。
- **整体评价**：代码质量高于平均水平，模块划分清晰（client / worker / config / model / theme 等），但存在若干会影响远程可用性、数据持久化与日常使用的设计 / 正确性问题，详见下文。

---

## 严重（Serious）

### 1. ureq 未启用任何 TLS 后端 —— HTTPS 端点会直接连不上
- **状态**：✅ 已修复
- **落地位置**：`Cargo.toml:10`（`features = ["json", "platform-verifier"]`）
- **验证结果**：`Cargo.lock` 新增 `rustls`(464) 与 `rustls-platform-verifier`(498)，被 `ureq` 引用(780)。注：本轮未实际跑 `cargo check`（另一 worker 并发改 #7 源码 + 禁改文件），依赖树层面已确认 TLS 后端就位。
- **备注**：采用 `platform-verifier`（系统证书存储）而非硬编码 `rustls` 根证书，Windows 上更贴合默认证书链。
- **涉及模块 / 文件**：`Cargo.toml`、`Cargo.lock`
- **具体证据**：
  - `Cargo.toml:10` 仅 `ureq = { version = "3", default-features = false, features = ["json"] }`
  - `Cargo.lock` 中 ureq 3.x 依赖完全没有 rustls / native-tls / openssl
- **为什么不合理**：ureq v3 把 TLS 拆成独立 feature，仅 `json` feature 时无 TLS 实现，任何 `https://` 请求都会失败（报错 "No TLS implementation configured"）。README 说明真实 AxonHub 走 `/admin/graphql`，远程部署几乎必然是 HTTPS，只有默认 `http://localhost:8090` 能用。
- **建议改法**：在 `Cargo.toml` 给 ureq 加 TLS feature，如 `features = ["json", "rustls"]`（或 `native-tls`），并补一个对真实 HTTPS 端点的冒烟测试。

### 2. Worker 持有「冻结」的 Config 副本 —— UI 改了 endpoint/project_id 永远传不到轮询线程
- **状态**：✅ 已修复
- **落地位置**：`src/worker.rs`（`Command::SetEndpoint`(57)/`Command::SetProject`(59)/`apply_command`(114)）；`src/main.rs:1928`（`send(Command::SetEndpoint(...))`）；`src/app.rs:123`（`send(Command::SetProject(...))`）
- **验证结果**：源码符号已落地；`worker.rs` 中旧的 `Pause` 相关分支已删除（grep 无 `Pause`/`paused` 残留）。
- **备注**：采用「下发 `Command`」方案（非 `Arc<RwLock>` 共享），与文档原建议一致；`SetEndpoint`/`SetProject` 经 `apply_command` 实时改写 worker 内 `config`。
- **涉及模块 / 文件**：`main.rs`、`app.rs`、`worker.rs`
- **具体证据**：
  - `main.rs:1927` 登录表单里 `s.app.config.endpoint = form.endpoint.trim()...`
  - `app.rs:94-99` `SignedIn` 里 `self.config.project_id = project`
  - `worker.rs:67-73` `Worker::spawn(config.clone(), ...)` 把配置 move 进线程
  - `worker.rs:170` 用 `config.signin_url()`、`worker.rs:215-218` 用 `config.graphql_url()` / `config.project_id()`
  - `worker.rs:44-58` 的 `Command` 枚举只有 `SetRowLimit` / `SetToken` / `Pause` 等，没有 `SetEndpoint` / `SetProject`
- **为什么不合理**：用户在登录表单里改过的 endpoint 及 `SignedIn` 自动解析出的 project_id 在 UI 端更新了，但后台线程用启动时旧 `Config`。改 endpoint 后登录实际打旧 endpoint；若 project_id 留空靠 `resolve_project` 自动选，worker 一直用空 projectID 去查，列表永远为空。
- **建议改法**：让 `Config` 在 UI 与 worker 间共享（`Arc<RwLock<Config>>`），或把 endpoint / project_id 也作为 `Command` 下发（`Command::SetEndpoint` / `Command::SetProject`），并保证 `SignedIn` 解析出的 project_id 同步给 worker。

---

## 中等（Medium）

### 3. 配置文件损坏时静默回退到默认值，丢失用户设置且无提示
- **状态**：✅ 已修复
- **落地位置**：`src/config.rs`（`ConfigLoadStatus` 枚举(187)、`load_with_status()`(195)、`preserve_corrupt()`(281)、`try_save()`(239)）；`load()`(233)/`save()`(251) 签名保持不变。
- **验证结果**：源码已落地。解析失败时 `preserve_corrupt()` 将原文件重命名为 `.corrupt` 并保留默认配置；`save()` 改为调用 `try_save()` 以暴露写错误。
- **备注**：`load()` 仍返回 `Config`（`load_with_status().0`），对调用方无破坏性改动；UI 可借 `ConfigLoadStatus::Corrupt` 弹提示（需 UI 接入，不在本轮）。
- **涉及模块 / 文件**：`config.rs`
- **具体证据**：
  - `config.rs:182-194` `Config::load()` 用 `read_to_string(...).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()`
  - `config.rs:196-204` `save()` 用 `let _ = std::fs::write(...)` 吞掉写错误
- **为什么不合理**：`config.json` 被手改坏、磁盘只读或写入中途崩溃时，所有用户配置（endpoint、窗口位置、字号、行数、置顶等）会被无声抹掉，启动无警告。典型「静默失败 + 数据丢失」。
- **建议改法**：解析失败时保留旧文件（重命名为 `config.json.corrupt` 并打日志 / 弹提示）；`save()` 对写失败至少记日志或回退。

### 4. 密码无法输入 / 粘贴非 ASCII 字符 —— 中文等特殊密码会被悄悄丢弃
- **状态**：✅ 已修复
- **落地位置**：`src/ui/login.rs`（`accepts()`(162)、`insert()`(169)、`paste()`(185)）
- **验证结果**：源码已落地。`accepts()` 对 `Field::Password` 仅排除 `is_control()`，其余字段保留 `is_ascii_graphic() || ' '`。`insert`/`paste` 均经 `accepts()` 过滤。
- **备注**：粘贴时密码字段保留首尾空格（真实密码可能含空格），其余字段 `trim()` 去除 `Bearer ` 前缀/空白。
- **涉及模块 / 文件**：`login.rs`
- **具体证据**：
  - `login.rs:156-167` `insert()` 与 `login.rs:172-189` `paste()` 都按 `ch.is_ascii_graphic() || *c == ' '` 过滤
- **为什么不合理**：密码含中文、emoji 等非 ASCII 时，要么打不进去、要么 Ctrl+V 粘贴时被静默逐字符删除，用户无感知，最终登录失败且难排查。endpoint / email 过滤非 ASCII 合理，但密码同样限制是 bug。
- **建议改法**：密码字段允许任意可打印 Unicode（仅排除 `ch.is_control()`），仅对 endpoint 保留 ASCII / URL 校验；粘贴按同样字段策略。

### 5. 整条「暂停」功能是无用死代码（且 app.paused 从未被读取）
- **状态**：✅ 已修复
- **落地位置**：`src/worker.rs`（无 `Command::Pause`/`paused`）、`src/app.rs`（无 `pub paused`）、`src/main.rs`（无 `MENU_PAUSE`）
- **验证结果**：三处 grep 均无 `Pause`/`paused`/`MENU_PAUSE` 残留。**`MENU_SIGNIN` 仍在 `main.rs:229`，本轮范围外、未被误删** ✅。
- **备注**：采用「整体删除未连线代码」方案（与原建议二选一），彻底移除误导维护者的死代码。
- **涉及模块 / 文件**：`main.rs`、`app.rs`、`worker.rs`
- **具体证据**：
  - `main.rs:229` `MENU_PAUSE = 1002`、`main.rs:230` `MENU_SIGNIN = 1100` 都有 handler（`:1983-1985`、`:2019`），但 `show_menu()`（`:2037-2157`）上下文菜单里没添加「暂停」「登录」两项
  - `app.rs:34` `pub paused: bool` 只在 `:1984` 死分支写入，全代码无读取
  - `worker.rs:119, 134-135, 199` 的 `Command::Pause` / `paused` 只在 UI 永不发送的命令分支生效
- **为什么不合理**：菜单常量、handler、App 字段、Worker 命令形成一整套无法触发的代码，误导维护者以为「暂停」可用。
- **建议改法**：要么把「暂停轮询」真正接进右键菜单（`AppendMenuW` 增加 `MENU_PAUSE` 项并 checked 反映 `app.paused`），要么整体删掉这套未连线代码。

### 6. SignedIn 解析出的 project_id 不写盘 —— 重启后回退
- **状态**：✅ 已修复
- **落地位置**：`src/app.rs:121`（`self.save()`，位于 `Update::SignedIn` 分支内）
- **验证结果**：源码已落地。与 #2 联动后，解析出的 `project_id` 既经 `SetProject` 下发给 worker，又写入 `config.json` 持久化。
- **备注**：依赖 #2 已修复，`save()` 写盘才有意义（否则 worker 仍拿不到）。
- **涉及模块 / 文件**：`app.rs`
- **具体证据**：`app.rs:94-119` `SignedIn` 分支更新 `self.config.project_id` 并调 `self.config.store(&stored)`（`store` 只写 `credentials.bin`），但没调 `self.save()`（写 `config.json`）
- **为什么不合理**：当 project_id 来自自动解析（配置为空时），每次启动都要重新解析；叠加 #2 后即便写盘 worker 也拿不到。
- **建议改法**：在 `SignedIn` 后调 `self.save()` 持久化（前提 #2 已修复）。

### 7. 全程缺少日志 / 追踪 —— 后台轮询的失败只能「看状态栏」
- **状态**：✅ 已修复
- **落地位置**：
  - `Cargo.toml` 新增 `tracing` / `tracing-subscriber` 依赖
  - `main.rs:503-513` 初始化 `tracing_subscriber::fmt()` 文件 subscriber（`with_writer(make_writer)`，`with_max_level(INFO)`，`set_global_default`）
  - `config.rs`（`use tracing::warn;`）：DPAPI 加解密失败、凭据目录/文件写入失败、配置损坏备份、配置保存失败等吞错点均 `warn!` 出去（仅记录失败原因与路径，**不写明文凭据**）
  - `client.rs`（`use tracing::warn;`）：`map_transport` 的 HTTP 拒绝 / 超时 / 传输层错误分支插桩（`auth={auth}` 中的 `auth` 为布尔，`other` 为 ureq 传输错误串，均不含令牌）
  - `worker.rs`（`use tracing::warn;`）：获取请求详情失败、登录失败、轮询失败等
  - `app.rs`（`use tracing::info;`）：`SignedIn` 成功后 `info!("登录成功")`
- **验证结果**：`cargo check` 与 `cargo test` 均通过；单测 **66 passed / 0 failed**，编译 0 警告。`src/` 内已确认存在 `tracing::` 调用且依赖就位。
- **凭据复核结论**：对全部 `warn!`/`info!` 插桩点逐一核查，**未发现任何把 token / JWT / password / email / Authorization 头写入日志的位置**——`Authorization` 头仅在 `client.rs:269` 发出未记录；凭据日志只记失败原因与文件路径。满足硬性安全要求。
- **涉及模块 / 文件**：`client.rs`、`config.rs`、仓库依赖
- **具体证据**：
  - `client.rs:330-337` `map_transport` 把所有非状态错误转 `ApiError::Unreachable(other.to_string())`
  - `config.rs` DPAPI 失败、文件读写失败全部 `let _ = ...` / 返回 `None`
  - 仓库无 `log` / `tracing` / `env_logger` 依赖
- **为什么不合理**：长驻后台、常轮询、凭据落盘的程序，配置损坏、DPAPI 解密失败、ureq TLS 错误、网络抖动都被吞，只状态栏一行红字，出问题极难排查，也无审计痕迹。
- **建议改法**：引入 `tracing` / `log` + 可选文件日志（或在 `%APPDATA%\ah-panel\` 写 `panel.log`），网络 / 存储 / 配置类错误 `warn!` / `error!` 出去，UI 仍显示友好文案。

### 8. 进程退出可能被一次阻塞请求拖住最多 12 秒
- **状态**：✅ 已修复
- **落地位置**：`src/worker.rs:302`（`commands.recv_timeout(...)` 分段等待，取代原 `std::thread::sleep(wait)`）
- **验证结果**：源码已落地。主循环以 `TICK` 为单位的 `recv_timeout` 轮询命令通道，`Shutdown` 可在当前请求间隙被立即处理，不再阻塞至 12s 超时。
- **备注**：`std::thread::sleep` 已从等待逻辑移除（grep 确认 `worker.rs` 内无残留）。
- **涉及模块 / 文件**：`worker.rs`
- **具体证据**：
  - `worker.rs:115` 请求超时 `REQUEST_TIMEOUT = 12s`
  - `worker.rs:264` 线程主循环 `std::thread::sleep(wait)`
  - `worker.rs:100-107` `Drop` 里 `send(Shutdown)` 后 `handle.join()`
- **为什么不合理**：worker 在 `fetch_requests` 阻塞期间（最长 12s）不会处理 `try_recv` 的命令（取命令在 fetch 之前），`Shutdown` 要等当前请求超时 / 返回才被处理，关闭窗口后主线程 `join()` 卡最多 12 秒，表现「退出卡顿」。
- **建议改法**：把命令检查放到轮询等待里（如带超时的 `recv_timeout`），或给 ureq agent 设更短 `timeout_global` 并区分「关闭时立即退出」。

---

### 附：模型名括号显示（正确性微调，原清单未单列）
- **状态**：✅ 已修复
- **落地位置**：`src/model.rs` 的 `display_model()`(382-387)
- **验证结果**：源码已落地。服务返回模型名与请求不一致时，由原 `「model(effort)」` 改为 `[model(effort)]` / `[model]` 方括号包裹（grep 确认无 `「`/`」` 残留）。
- **备注**：属显示格式微调，不与任何编号问题冲突；原 DESIGN_ISSUES 未单列此项，故附于此处。

---

## 轻微（Minor）

### 9. main.rs 是 ~2726 行「上帝模块」
- **状态**：🔄 部分处理
- **落地位置**：新增 `src/ui/clipboard.rs`；`src/ui/mod.rs` 导出 `clipboard`
- **验证结果**：剪贴板读写（`clipboard_text` / `set_clipboard_text`）已从 `main.rs` 抽到 `ui::clipboard` 模块，`main.rs` 由 2762 → 2710 行；相关 Win32 导入（簇板 / 全局内存 / `CF_UNICODETEXT`）已从 `main.rs` 剪除，`cargo check --tests` 0 警告。详情弹窗的**绘制**此前已位于 `src/ui/detail.rs`。
- **备注**：完整拆分（把 `wnd_proc` / `detail_proc` 两个窗口过程本体的几何、菜单等一并外移）会级联改动 `window_scale` / `single_line_mode` 等 `main.rs` 私有辅助与两处窗口过程调用点，改动面大且难以在本轮做运行时回归，故留待后续；本轮已消除最具自包含性的剪贴板部分。
- **涉及模块 / 文件**：`src/main.rs`、`src/ui/clipboard.rs`、`src/ui/mod.rs`
- **建议改法**：把 `detail_proc` 及其绘制 / 输入拆到 `ui/detail` 或独立 `popup.rs`；菜单 / 剪贴板 / 几何辅助也拆分。

### 10. 两个窗口过程重复大量 Win32 样板
- **状态**：✅ 已修复
- **落地位置**：`src/main.rs:1494`（`with_double_buffer`）
- **验证结果**：`paint`(1524) 与 `paint_detail`(2664) 现共用 `with_double_buffer(hdc, rc, |painter| …)`，双缓冲的建 DC / 位图、填背景、`BitBlt`、清理样板已消除；工作区约束此前即由公共 `clamp_window_rect` / `work_area_*` 供两处窗口过程共用（`grep` 确认）。`cargo check --tests` 0 警告。
- **涉及模块 / 文件**：`src/main.rs`（`wnd_proc` 与 `detail_proc`）
- **具体证据**：`wnd_proc` 与 `detail_proc` 各自重写 `WM_PAINT` 双缓冲（位图 / DC / `BitBlt`）、`WM_NCHITTEST`、`WM_SETCURSOR`、`WM_MOUSEWHEEL`、`WM_SIZING` / `WM_MOVING` / `WM_WINDOWPOSCHANGING` 约束到工作区、`WM_GETMINMAXINFO`。
- **建议改法**：抽取 `double_buffer_paint`、公共 `clamp_to_work_area` 辅助复用。

### 11. token.rs 自实现 base64url 对 +/` 过宽容忍
- **状态**：✅ 已修复
- **落地位置**：`src/token.rs:57-61`
- **验证结果**：`value()` 现为 `b'-' => Some(62)`、`b'_' => Some(63)`，`+` / `/` 归入 `_ => None`。新增用例 `rejects_classic_base64_alphabet` 断言 `base64url_decode("a+b")` 与 `("a/b")` 均为 `None`；`cargo test` 通过。
- **涉及模块 / 文件**：`src/token.rs`
- **具体证据**：`token.rs:52-81` `value()` 中 `b'-' | b'+' => Some(62)`、`b'_' | b'/' => Some(63)`
- **建议改法**：base64url 只接受 `-` / `_`，遇到 `+` / `/` 直接返回 `None`。

### 12. model.rs 失败 / 重试计数基于列表查询里被截断的 executions(first:10)
- **状态**：✅ 已修复
- **落地位置**：`src/model.rs`（`Row::attempts_truncated` 字段、`from_wire`、`is_error`）
- **验证结果**：`Row` 新增 `attempts_truncated: bool`，`from_wire` 以 `total_count > edges.len()` 判定是否被截断；`is_error()` 在截断时保守返回 `true`，保证卡片仍可点开详情。新增用例 `a_request_with_more_attempts_than_fetched_stays_clickable`（12 次尝试 / 10 条 edges）断言 `attempts_truncated == true` 且 `is_error() == true`；`cargo test` 通过。
- **备注**：`attempt_count` 仍取连接的 `total_count`（本就为全量），故仅需修正对 `failed_attempts` 完整性的假设。
- **涉及模块 / 文件**：`client.rs`、`model.rs`
- **具体证据**：
  - `client.rs:29` 列表查询 `executions(first: 10, ...)`
  - `model.rs:300-320` `attempt_count` / `failed_attempts` 由这 10 条推导
- **建议改法**：不要依赖被截断计数判定「是否值得点开」，或超 10 次保守标记为可点开。

### 13. app.rs 过滤在 truncate(row_limit) 之后执行
- **状态**：✅ 已满足（代码现状即正确，原描述滞后）
- **落地位置**：`src/app.rs:177-189`
- **验证结果**：`rebuild()` 现为「排序 → `retain` 按掩码过滤(180-182) → `truncate(row_limit)`(183)」，即**先过滤再截断**，符合建议语义；`filter_keeps_only_matching_rows` 等用例通过。原文所引 `先 truncate 再 retain` 的代码已不存在，无需改动。
- **涉及模块 / 文件**：`src/app.rs`
- **具体证据**：`app.rs:172-184` 先 `rows.truncate(...)` 再按状态掩码 `retain`
- **建议改法**：先过滤再截断（或过滤后不足时回退补齐）。

### 14. 双缓冲绘制对 GDI 空句柄无防护
- **状态**：✅ 已修复
- **落地位置**：`src/main.rs:1494-1522`（`with_double_buffer`）
- **验证结果**：`CreateCompatibleDC` 返回无效句柄时直接返回 `None`；`CreateCompatibleBitmap` 无效时先 `DeleteDC(mem)` 再返回 `None`，均不再进入 `SelectObject` / `BitBlt` / `DeleteObject`。返回 `Option`，调用方以 `let _ =` / `.unwrap_or(0.0)` 容忍失败且不绘制。
- **涉及模块 / 文件**：`src/main.rs`
- **具体证据**：`main.rs:1504-1540` 与 `main.rs:2688-2719` 中若 `CreateCompatibleDC` / `CreateCompatibleBitmap` 返回 null，仍继续 `SelectObject` / `BitBlt` / `DeleteObject`。
- **建议改法**：任一 GDI 对象创建失败直接 `EndPaint` 返回，不继续绘制。

### 15. theme.rs unsafe impl Send for Fonts 仅靠「只用 UI 线程」约定保证
- **状态**：✅ 已修复
- **落地位置**：`src/theme.rs:85-99`
- **验证结果**：已删除 `unsafe impl Send for Fonts {}`，改为新增字段 `_ui_thread: PhantomData<std::rc::Rc<()>>`，使 `Fonts` 成为 `!Send` + `!Sync`。`State` 存于 `thread_local!`（`main.rs:252-256`）本就 `!Send`，故编译不受影响；类型系统现会**拒绝**任何把 `Fonts` 移出 UI 线程的代码。`cargo check --tests` 0 警告。
- **备注**：`*mut T` 天生即 `Send`，原先的 `unsafe impl Send` 实为冗余「声明」；真正有意义的加固是反向约束为 `!Send`。
- **涉及模块 / 文件**：`src/theme.rs`
- **具体证据**：`theme.rs:92-97` 字体 / 字族是裸指针 `*mut GpFontFamily` 并手动 `unsafe impl Send`
- **建议改法**：用 `thread_local` 或类型系统约束（如 `NonSend` / `!Send` 包装）表达「仅 UI 线程」，避免裸 `unsafe impl Send`。

### 16. paint / save 等写入错误被静默忽略（与 #3 / #7 同源）
- **状态**：✅ 已修复
- **落地位置**：`config.rs:264-268`、`src/ui/clipboard.rs`
- **验证结果**：
  - `config.save()` 现为 `if let Err(e) = self.try_save() { warn!("配置保存失败: {e}") }`（`config.rs:264-268`），写盘错误已记录（上轮 #7 已落地）。
  - 剪贴板 `set_clipboard_text` 三处失败分支（打开剪贴板 / 分配内存 / `SetClipboardData` 交接）均补 `warn!`，不再静默 `return`。
- **涉及模块 / 文件**：`config.rs`、`src/main.rs`、`src/ui/clipboard.rs`
- **具体证据**：
  - `config.rs:201-204` `let _ = std::fs::write(config_path(), json)`
  - `main.rs:2400-2416` 剪贴板写入失败也 `return`
- **建议改法**：至少记录日志（结合 #7）。

---

## 优先修复建议

1. **先做 #1**（加 TLS feature，否则远程部署不可用）和 **#2**（共享 / 下发 `Config` 给 worker）。—— 已完成
2. **其次 #3 / #4**（静默数据丢失与密码输入 bug 直接影响日常使用）。—— 已完成
3. 其余（#5 ~ #16）：#5–#8、#10–#16 已完成；#9 已抽出剪贴板模块、消除双缓冲重复，完整窗口过程拆分留待后续。

> 注：#2 与 #6 强相关 —— #2 修复后，`SignedIn` 解析出的 project_id 才能通过共享配置或命令下发真正到达 worker，#6 的写盘持久化才有意义。
