# AH Panel

AxonHub / Octopus 请求监控面板 — 常驻桌面右上角的悬浮小窗,自动轮询并实时显示最近的请求,
无需打开浏览器。

## 特性

- **双网关**:可同时显示 AxonHub 与 [Octopus](https://github.com/bestruirui/octopus) 两个网关的
  请求,按时间混排;列表混有两个来源时,卡片行首标 `AH` / `OCT` 来源徽章。
- **低占用**:GDI+ 自绘,常驻约 15 MB,空闲 CPU 0%。对比:WebView2 方案约 180 MB、egui/wgpu 约 276 MB。
- **实时**:有请求进行中时 2 秒轮询,空闲时 5 秒;服务异常时指数退避(2s → 30s),不会打爆网关。
- **不抢焦点**:列表视图以 `WS_EX_NOACTIVATE` 显示,瞄一眼不会夺走你的输入焦点;
  仅在登录表单显示时才会激活窗口以接收键盘输入。
- **多显示器物理尺寸自适应**:字体与布局按当前屏幕的*物理像素密度*(EDID 报告的每英寸像素数)缩放,
  而非仅按 Windows 的缩放设置。两块都设为 100% 但密度不同的屏幕(例如 1080p 与 4K),
  旧实现会在 4K 屏上显得偏小;现在面板在任何屏幕上都是同一物理尺寸。在密度不同的屏幕间拖动时
  自动重建字体、按比例缩放窗口并重新计算卡片尺寸;同缩放但不同密度的跨越由 `WM_MOVE` 单独侦测
  (`WM_DPICHANGED` 只在系统 DPI 设置变化时触发)。窗口尺寸以 96 DPI 逻辑单位保存,因此在任何分辨率
  的屏幕上打开都是同样的观感大小;窗口位置与工作区约束都基于它所在的那块显示器,不会被拉回主屏。
- **单一等宽字体**:中英文统一用 `JetBrainsLxgwNerdMono`(缺失时依次回退
  `Maple Mono NF CN` / `Microsoft YaHei UI` / `Segoe UI`)。它是等宽的,
  中文 `汉字宽度 = 2 × 英文宽度`,列对齐因此是精确的。
- **令牌优先**:可直接粘贴访问令牌使用,不必把账号密码交给面板;凭据均以 Windows DPAPI 加密(绑定当前用户),也可完全不保存。

## 使用

```
cargo run --release
```

首次启动弹出登录表单,有两种方式:

### 方式一:直接粘贴访问令牌(推荐)

在浏览器打开 AxonHub,打开开发者工具 → Network,随便点一个 `/admin/graphql`
请求,复制请求头里的 `Authorization: Bearer eyJ...`,粘贴到表单的「访问令牌」页签。

不会保存账号密码,只保存令牌。令牌有效期 7 天,过期后重新抓一个即可。

也可以用环境变量传入,完全不落盘:

```powershell
$env:AXONHUB_ACCESS_TOKEN = "eyJhbGciOi..."
cargo run --release
```

环境变量优先于本地保存的令牌。变量名可在配置里改(`tokenEnvVar`)。

### 方式二:账号密码

切到「账号密码」页签填写。默认也只保存令牌,不保存密码。

### 凭据保存方式

表单上的「凭据:」一行点击可循环切换三种模式:

| 模式 | 行为 |
|---|---|
| `token`(默认) | 仅加密保存访问令牌,不保存密码;7 天后需重新登录 |
| `password` | 加密保存邮箱和密码,自动静默登录,永不打扰 |
| `none` | 什么都不保存,每次启动重新输入 |

全部使用 **Windows DPAPI** 加密(绑定当前用户账户),存放在
`%APPDATA%\ah-panel\credentials.bin`。

### 操作

| 操作 | 说明 |
|---|---|
| 拖动标题栏/空白区域 | 移动面板 |
| 拖动边缘 | 调整窗口大小(高度变化后自动调整拉取条数) |
| 单击卡片 | 在浏览器中打开该请求详情 |

> 详情页链接为 `/project/requests/{URL 编码后的 GUID}`,例如
> `http://localhost:8090/project/requests/gid%3A%2F%2Faxonhub%2FRequest%2F34009`。
> 该路由要求完整 GUID 且必须编码:`:` 和 `/` 不编码会落到 404,并提示
> `guid must start with gid://axonhub/`。
| 滚轮 | 上下滚动 |
| 右键 | 刷新 / 打开请求页 / Octopus 令牌与面板 / 显示数量 / 字号 / 单行模式 / 清除凭据 / 固定窗口位置 / 置底 / 退出 |

> 卡片区域不参与拖动,点卡片是「打开该请求」;窗口边缘 6px 内是缩放区。

状态由卡片左侧色条表示,不显示文字标签:

| 颜色 | 状态 |
|---|---|
| 绿 `#3FB950` | 已完成 |
| 蓝 `#4C9AFF` | 处理中 |
| 红 `#F85149` | 失败 |
| 灰 `#8B949E` | 已取消 |
| 黄 `#D29922` | 等待中 |

## 卡片信息

每张卡片三行,对应 AxonHub 请求页的列:

1. `#请求号` · 相对时间 · 成本
2. 模型(**只显示实际服务的模型**;若与请求的模型不同则为金色 `#E8B33A` 以提示发生了路由)
   · 推理强度 · 协议 · 透传标记 · 重试次数

   协议单元格:两侧相同显示 `messages`(灰),发生转换显示 `messages→chat`(**金色**),
   任一侧缺失则显示已知的那一侧(灰,不声称"一致")。透传为真时附加金色 `透传` 徽章
   —— 为假时不显示,因为「未透传」是常态,标出来只是噪音。
3. 渠道 · 缓存命中率(命中率低且 prompt ≥ 40k 时标红)· 词元 · `总耗时 / 首字耗时`
   · **调用方**(最右,API key 名 + 所属用户,如 `cc · caolib cao`)

### 单行模式

右键「单行模式」把每张卡片压成**一行**:卡片高度由 44px 降到 26px,面板保持显示条数不变、
按新高度等比变矮;再点一次恢复两行。一行内的字段从左到右依次为:

```
模型(发生路由时套「」)→ 流/转/透 → 协议 → 调用方 → 渠道 → 词元 → 缓存命中率
    → 重试徽章 → TPS → 来源徽章 → 相对时间
```

一行放不下全部字段(面板宽度有限),所以按上面的优先级从左往右排,**放不下的项直接省略**
—— 不换行、不缩小字号,右侧的耗时与徽章永远保留。窗口高度会随模式切换重算,
开启后被拖拽缩放时,条数也仍按单行卡片计算。

### 相对时间

`刚刚` / `12 秒前` / `3 分钟前` / `2 小时前` / `昨天 11:20` / `3 天前`,
超过 7 天回退为绝对时刻(此时「12 天前」已不如直接看时间)。面板每秒重绘一次以保持
新鲜;若与 AxonHub 存在时钟偏差(算出负数)则回退为绝对时刻,不会显示「未来」。

> 没有底部状态栏,列表直接延伸到窗口底边。运行时状态(已登录 / 更新于 / 出错原因)
> 显示在标题下方那一行;出错时该行整行变红。

### 列对齐

第三行用**字符网格**排布而非估算像素:先量出等宽字体的单字符宽度,再按字符数摆放
`渠道` / `缓存` / `词元` 三列(18 / 10 / 之后剩余的字符位)。因为字体是等宽的,同列在
所有卡片上像素一致。第一行同理:`#编号` 占固定槽位,`流式` 与其后的相对时间因此不会
随编号长度左右跳动;该行所有单元格共用同一字体与同一矩形,基线一致。

## 配置

`%APPDATA%\ah-panel\config.json`,首次运行自动生成:

```jsonc
{
  "endpoint": "http://localhost:8090",
  "projectId": "gid://axonhub/Project/1",
  "pollSeconds": 5,          // 空闲轮询间隔
  "activePollSeconds": 2,    // 有请求进行中时的间隔
  "rowLimit": 12,            // 由窗口高度自动推导
  "credentialMode": "token", // token | password | none
  "tokenEnvVar": "AXONHUB_ACCESS_TOKEN",
  "octopusEndpoint": "http://localhost:8091", // 留空关闭第二个数据源
  "octopusTokenEnvVar": "OCTOPUS_AUTH_TOKEN",
  "alwaysOnTop": true,
  "pinPosition": false,      // 固定位置:开启后不可拖动、不可改大小
  "alwaysOnBottom": false,
  "fontSize": 12.5,          // 正文字号(96 DPI 基准像素);右键「字号」可在 10–24 间调整,整面板等比缩放
  "singleLine": false,       // 单行模式:每张卡片一行、面板更矮;右键「单行模式」切换
  // width/height 是 96 DPI 逻辑单位,与显示器缩放无关;x/y 是屏幕坐标(设备像素)。
  "window": { "x": -1, "y": -1, "width": 452, "height": 602 }
}
```

`x`/`y` 为 `-1` 表示首次运行,自动放到工作区右上角。

## 认证说明

AxonHub 的 `/admin/graphql`(请求列表的数据源)**只接受 JWT**。

**API key 用不了**,这一点容易踩坑:

| 接口 | 认证要求 | 用 `ah-...` key 的结果 |
|---|---|---|
| `/admin/graphql` | 仅 JWT | `401 Invalid token` |
| `/openapi/v1/graphql` | API key,且必须是 `service_account` 类型 | `401 Invalid API key` |
| `/v1/*`(调 LLM) | API key | 可用 |

`/openapi/v1/graphql` 即使换成 `service_account` key 也没用 —— 它的 schema 里只有
`apiKey` 和 `apiKeyQuotaUsages` 两个查询,**没有请求列表**。代码中也不存在
「API key 换 JWT」的接口,签发 JWT 的地方只有登录、邀请注册、OIDC 三处。

JWT 有效期 7 天(`biz/auth.go`),且没有 refresh token。面板会在启动时解析 `exp`
并提前提示是否过期,避免白白发一次必然 401 的请求。

## Octopus 数据源

面板可同时显示 AxonHub 与 [Octopus](https://github.com/bestruirui/octopus) 两个网关的请求,
按时间混排;列表同时含两个来源时,卡片行首显示 `AH` / `OCT` 来源徽章。

数据来源是 Octopus 的实时日志流 `GET /api/v1/log/overview/stream`(SSE):连接后立即下发
内存中最近约 50 条已结束请求与全部进行中请求,之后按请求增量推送。面板每次轮询读取约
1 秒后断开,下次连接用新快照覆盖,状态不会丢。该接口**只认登录换来的 `auth` Cookie**,
不接受 `sk-octopus-` 开头的 API key。

### 获取并粘贴令牌

1. 浏览器登录 Octopus(建议勾选「信任设备」,令牌有效期 30 天),打开开发者工具 →
   Application → Cookies → 复制 `auth` 的值;
2. 面板右键 → 「粘贴 Octopus 令牌」。剪贴板内容(`auth=…`、完整 Cookie 行或裸值均可)
   校验 JWT 有效期后加密保存(DPAPI);启动时也会读环境变量 `OCTOPUS_AUTH_TOKEN`(不落盘),
   变量名可在配置里改(`octopusTokenEnvVar`)。
3. 右键菜单会显示连接状态(未设置令牌 / 已连接 / 令牌无效或已过期),并可打开 Octopus
   面板首页或清除令牌。

### 字段差异

| 字段 | Octopus | 面板表现 |
|---|---|---|
| 首字耗时 | 无 | TPS 以总耗时近似,略偏低 |
| 推理强度 / 透传 / 流式 | 无 | 不显示对应标记 |
| 协议 | 数值位值(chat / responses / messages) | 与 AxonHub 同样参与「转」判断 |
| 重试轮次 | `round` | 行尾 `n×` 重试徽章 |
| 缓存命中 | `prompt_tokens_details.cached_tokens` | 命中率超过 100% 时按 100% 显示 |

> Octopus 的日志保存在内存,没有历史查询接口;重启 Octopus 后面板列表从空开始。
> 单请求没有可打开的详情页(网页端日志页是状态驱动的单页应用,不经过 URL),
> 点击 Octopus 卡片会打开其面板首页。

## 数据来源

复用 AxonHub 前端 `GetRequests` 的字段选择:

```
POST {endpoint}/admin/graphql
Authorization: Bearer <JWT>
X-Project-ID: gid://axonhub/Project/1

query GetRequests($first, $where, $orderBy) { requests(...) { ... } }
```

> `modelID`、`apiKey`、`clientIP` 是全大写缩写,不是标准 camelCase;
> `serde(rename_all = "camelCase")` 会静默地把它们变成 `None`,故需显式 `rename`。

## 结构

| 文件 | 职责 |
|---|---|
| `main.rs` | 窗口生命周期、消息循环、输入分发 |
| `app.rs` | 面板状态 |
| `worker.rs` | 后台轮询线程 |
| `client.rs` | GraphQL / 登录 HTTP |
| `octopus.rs` | Octopus SSE 日志流(读快照、按 id 合并) |
| `model.rs` | 数据结构与行转换 |
| `config.rs` | 配置读写、凭据模式与 DPAPI 存储 |
| `token.rs` | JWT 有效期解析 |
| `theme.rs` | 配色、GDI+ 绘制原语、中英文分段绘制(`split_runs`) |
| `ui/layout.rs` | 几何布局(纯计算,含 DPI 缩放,可测) |
| `ui/panel.rs` | 请求卡片渲染 |
| `ui/login.rs` | 登录表单 |

### 两个实现要点

- **双缓冲**:`WM_PAINT` 先画到离屏位图再一次性 `BitBlt`。直接画窗口 DC 会在每次
  重绘时露出半成品画面,表现为闪烁。
- **副作用延迟执行**:`ShellExecuteW`、`SetWindowPos`、`DestroyWindow` 都会泵消息、
  重入窗口过程。窗口过程把这类调用收集到 `Vec<Action>`,在释放状态借用后再执行,
  否则会触发 `RefCell already borrowed`。
