# CODEBUDDY.md

This file provides guidance to CodeBuddy Code when working with code in this repository.

## 构建、运行与测试

本仓库是纯 Windows 二进制 crate，依赖 `windows` 0.62（GDI+ / GDI / DPAPI / Win32 消息循环）。**只能在 Windows 上编译和运行**。

```bash
cargo run --release          # 运行（首次启动弹登录窗）
cargo build --release        # 仅构建；产物在 target/release/ah-panel.exe
cargo test                   # 跑全部内置测试（app/config/format/model/theme 的 mod tests）
cargo test --bin ah-panel <过滤>   # 例：cargo test --bin ah-panel model:: 只跑某模块
cargo clippy --all-targets   # 代码质量检查（需已装 clippy）
cargo fmt                    # 格式化
```

测试全部以内联 `#[cfg(test)] mod tests` 形式写在各源文件里（无 `dev-dependencies`，纯 std），`cargo test` 直接跑。注意 `Worker::run` 涉及真实网络，测试只覆盖纯逻辑（解析、布局、格式、配置）。

`Cargo.toml` 的 `[profile.release]` 设了 `opt-level="z"`、`lto=true`、`panic="abort"`、`strip=true`——发布构建体积敏感，改 profile 需谨慎。

## 配置与数据位置

- 配置：`%APPDATA%\ah-panel\config.json`，首次运行自动生成（`config.rs:338`）。
- 凭据（DPAPI 加密，绑定当前用户）：`%APPDATA%\ah-panel\credentials.bin`。
- 用环境变量 `AH_PANEL_HOME` 可整体覆盖该目录（`config.rs:322`），用于指向不同 profile 或本地测试。
- 刷新间隔由 `pollSeconds`（空闲，默认 5）和 `activePollSeconds`（有请求 in flight，默认 2）控制，**设置窗口里没有这一项**，只能改 `config.json` 后重启。

## 高层架构

### 1. 线程模型与数据流（双通道解耦）

单条 UI 线程跑 Win32 消息循环（`main.rs`）；另一条 `ah-panel-poll` 线程跑网络轮询（`worker.rs:run`，由 `Worker::spawn` 起）。两者用两个 `mpsc` 通道解耦，互不阻塞：

- 主 → worker：`Worker::send(Command)`（`worker.rs:141`），命令如 `RefreshNow` / `SetTargets` / `FetchDetail` / `TestAccount`。
- worker → 主：`Worker::drain()`（`worker.rs:146`），**非阻塞**拉取自上次的全部 `Update`（`SignedIn` / `Snapshot` / `Accounts` / `Detail` / `Probe` / `Failed` / `SignedOut`）。

UI 线程上每 200ms 的 `WM_TIMER`（定时器 id `TIMER_POLL`）触发 `pump_worker`（`main.rs:1765`）：`drain()` 取出 Update → 经 `State::with` 应用到 `App` → 把 `Action::Redraw` 压入待执行队列。**真正的网络轮询间隔由 worker 自己按 config + backoff 计算**（`worker.rs:426-436`），和这 200ms 的 UI 同步节拍是两回事。

### 2. 全局可变状态 `State`

整个面板状态装在一个 `thread_local! RefCell<Option<State>>`（`main.rs:218-219`），结构见 `main.rs:246`：`app`（面板数据）、`worker`、`fonts`、`detail`（错误详情弹窗，至多一个）、`settings`/`settings_login`（设置窗口往返）、`activatable`、`scale`（绘制缩放比）。所有窗口过程都通过 `State::with(|s| ...)`（`main.rs:268`）借 `&mut` 改状态。因为是单 UI 线程，`RefCell` 是安全的——不要试图把 `State` 搬到别的线程。

### 3. 副作用延迟执行：Action 机制

`ShellExecuteW` / `SetWindowPos` / `DestroyWindow` 这类 Win32 调用会泵消息、重入窗口过程，若在持有 `RefCell` 借用时调用会触发 `RefCell already borrowed`。所以窗口过程**不直接**执行这些副作用，而是把意图收集进 `Vec<Action>`（`main.rs:1266`），在释放状态借用之后由 `run_actions`（`main.rs:1316`）统一执行。Action 还能链式再压 Action（函数用 `while let Some ... pop` drain 而非单次迭代）。任何新增会触发窗口/进程级副作用的逻辑，都应走 `Action`，不要就地调用 Win32。

### 4. 绘制管线：双缓冲 + 物理 DPI 缩放

- `WM_PAINT` 经 `with_double_buffer`（`main.rs:1647`）先画到离屏位图再一次性 `BitBlt`，避免闪烁。
- 布局计算集中在 `ui/layout.rs`（纯函数、可测），按**显示器物理像素密度**（EDID，非系统缩放）缩放；窗口尺寸以 96 DPI 逻辑单位保存（`config.window`），故跨屏观感大小一致，`WM_DPICHANGED` 只在系统 DPI 变化时触发，跨屏密度不同由 `WM_MOVE` 单独侦测。
- 中英文字混排由 `theme.rs` 的 `split_runs` 分段绘制（中文缺字形时回退字体）。卡片第三行用**字符网格**（先量等宽字体单字宽，再按字符数摆列，而非估算像素）保证列对齐——前提是字体等宽（见 README 字体约束）。

### 5. 数据模型与显示

- `model.rs` 把 GraphQL 响应转成 `Row` / `ExecutionDetail`，是列表与详情弹窗共享的数据结构。
- `token.rs` 解析 JWT `exp`，启动即提示是否过期，避免白白发一次必 401 的请求（AxonHub 令牌 7 天有效、无 refresh）。
- `config.rs` 负责配置读写 + 三种凭据模式（token/password/none）+ DPAPI 存储。

### 6. 认证约束（易踩坑）

`/admin/graphql`（列表数据源）**只接受 JWT Bearer**，不接受 API key（会 `401 Invalid token`）。代码里没有「API key 换 JWT」的接口；签发 JWT 只有登录 / 邀请注册 / OIDC 三处。给面板配凭据时务必用访问令牌或账号密码登录，而非 API key。

## 跨文件陷阱 / 注意点

- **serde 字段重命名**：AxonHub 部分字段是全大写缩写（`modelID`、`apiKey`、`clientIP`、`requestURL`、`baseURL`），`serde(rename_all="camelCase")` 会把它们静默变成 `None`。这些字段必须显式 `#[serde(rename=...)]`，新增 GraphQL 字段时务必核对大小写。
- **两层定时器**：200ms 的 `TIMER_POLL` 只负责把 worker 结果同步进 `State` 并触发重绘；网络轮询间隔由 worker 自管。改刷新频率别动 UI 定时器，改 `config` 的 `pollSeconds` / `activePollSeconds`。
- **弹窗共享 State**：detail / settings 是独立窗口但都通过同一个 `State::with` 访问状态。`Action::Redraw` 只作用于发起它的那个 `hwnd`，所以 `pump_worker` 在 `Popup` 打开时会额外 `invalidate(popup_hwnd)`（`main.rs:1791` 附近），否则弹窗不会刷新。
- **运行中的 exe 会锁住 release 构建**：改完代码想 `cargo build --release` 前，先把正在跑的 `ah-panel.exe` 关掉，否则 Windows 文件锁会导致链接失败。
