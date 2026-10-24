# Hot-RS-Launcher TUI

`hot-rs-launcher tui` 是同一个 CLI 二进制里的全屏终端前端。它不是 GUI，也不替代命令行：所有下载、校验、账号存储、加载器安装和启动逻辑仍然来自原来的 CLI 核心；TUI 只是把这些能力组织成一个可键盘和鼠标操作、带实时进度和动效的界面。

```sh
hot-rs-launcher tui
hot-rs-launcher ui          # 别名
hot-rs-launcher --dir ./minecraft tui
```

需要交互式终端（stdin/stdout 都是 TTY）、ANSI 光标寻址能力和 UTF-8 字体；`TERM=dumb` 会被拒绝。在管道、CI 或 `--help` 式重定向下运行会直接报错退出，不会挂起。

## 界面

```text
 ◆ Hot-RS-Launcher  v2.6.0                 ◆ cli-first                     ● Steve  ·  1.21.4 · Fabric  · auto
──────────────────────────────────────────────────────────────────────────────────────────────────────────────
╭────────────────────╮ ╭ INSTANCE ───────────────────────────────────────────────────────────────────────────╮
│      ▄██████▄      │ │M I N E C R A F T                                 ╭─────────────────────────────────╮│
│     ██  ██  ██     │ │C L I   ·   L A U N C H E R                       │             ▶  PLAY             ││
│     ████▀▀████     │ │                                                  │           press enter           ││
│     ████  ████     │ │◆ 1.21.4  ·  Fabric  ·  1 mod                     ╰─────────────────────────────────╯│
│     ▀▀██████▀▀     │ │◉ Steve  ·  OFFLINE                                                                  │
│   Hot-RS-Launcher  │ │                                                  RAM  4 GB                          │
│       v2.6.0       │ │                                                  LOAD Fabric                        │
│                    │ │                                                  PATH ~/minecraft                   │
│▌ ◈ Dashboard    1  │ ╰─────────────────────────────────────────────────────────────────────────────────────╯
│                    │                                                                                        
│  ▤ Versions        │ ╭ VERSIONS ──────────╮╭ MODS ACTIVE ───────╮╭ MEMORY ───────────╮╭ JAVA ──────────────╮
│                    │ │1                   ││1                   ││4 GB               ││detecting           │
│  ❖ Mods            │ │1.21.4              ││1 disabled          ││16 threads         ││scanning system…    │
│  ▣ Content         │ │                    ││                    ││                   ││                    │
│  ◉ Accounts        │ ╰────────────────────╯╰────────────────────╯╰───────────────────╯╰────────────────────╯
│                    │                                                                                        
│  ✧ Settings        │ ╭ RECENT ACTIVITY  ·  l for the full log ─────────────────────────────────────────────╮
│                    │ │00:00 · Hot-RS-Launcher TUI ready                                                    │
│  ≡ Activity        │ │00:00 · Game directory: ~/minecraft                                                  │
│                    │ │00:00 · 1 version(s) installed · source auto                                         │
│  ✦ About           │ │00:00 · Version 1.21.4 detected                                                      │
│                    │ │00:00 ✓ Fabric profile ready                                                         │
│                    │ │00:00 ! A mod is disabled                                                            │
│                    │ │00:00 ✕ Example error entry                                                          │
│                    │ │00:00 · internal detail                                                              │
│ ◈ 4 GB  16 threads │ │                                                                                     │
│ ◆ 1.21.4  Fabric   │ │                                                                                     │
│ ↻ 0.0s             │ │                                                                                     │
╰────────────────────╯ ╰─────────────────────────────────────────────────────────────────────────────────────╯
 enter  play   n  new version   i  loader   s  search mods   tab  switch   ?  help   q  quit                  
 ● READY  ~/minecraft                                                        src auto  ·  ↻ 0.0s  ·  02:10 UTC
```

窗口宽度小于 96 列时自动切换为顶部标签页布局；小于 52×16 时显示可读的尺寸提示而不是崩溃。所有界面使用真彩色，并在窄终端中裁剪而不是溢出。

## 功能

| 区域 | 能力 |
| --- | --- |
| Dashboard | 活跃实例、账号、加载器、RAM、Java、版本/模组统计；一键启动；空目录引导 |
| Versions | 本地版本列表、详情（基准版本、Java 要求、磁盘占用）、设为活跃、启动、下载远程版本、选择/安装 Fabric/Forge/NeoForge、重命名/复制/删除实例、导出 .mrpack |
| Mods | 已安装模组列表、启用/禁用、卸载、按当前版本与加载器搜索 Modrinth、安装、检查/应用 Modrinth 更新（`↑` 标记） |
| Content | 存档列表（名称、版本、模式、大小、图标）与备份/导入/删除；资源包列表（启用状态、pack_format、描述）与启停/导入/删除；`t` 切换两类，`o` 打开目录 |
| Accounts | 账号列表、默认账号、离线账号、Microsoft 浏览器/设备码登录、登出 |
| Settings | 界面语言（English / 中文）、内存、窗口尺寸、下载线程、官方/BMCLAPI 源、Java 运行时选择（带推荐项）、JVM 参数、GC 预设、减少动效、开场动画 |
| Activity | 完整活动日志、实时追踪、暂停跟随、清空 |
| 关于（About） | 版本、作者、GitHub（@Hotsteel2901）与 Codeberg（@hotsteel）仓库地址、许可证；`g` / `b` 在浏览器中打开 |
| 全局 | 后台任务面板、分阶段进度、实时传输速度、每文件百分比、取消任务、Toast、帮助；底部状态栏与任务面板 |

## 快捷键

### 全局

| 按键 | 行为 |
| --- | --- |
| `Tab` / `Shift+Tab` | 下一个 / 上一个界面 |
| `1` … `8` | 直接切换界面 |
| `↑` `↓` | 移动选择 |
| `Enter` | 执行当前界面的主要动作 |
| `Esc` | 返回 Dashboard；关闭弹窗；关闭已完成的任务面板 |
| `?` | 帮助 |
| `j` | 显示/隐藏任务面板 |
| `c` | 取消正在运行的任务（任一界面） |
| `q` / `Ctrl+C` | 退出（终端会恢复；运行中的任务会被取消） |

### 鼠标

- 点击开场动画可跳过；点击侧栏或顶部标签切换界面；点击 Dashboard 主按钮、统计卡和最近活动面板执行对应动作。
- 点击列表行选中；双击同一行执行主要动作（启动版本、启停模组、设为默认账号、安装搜索结果、确认选择器条目）。
- 点击设置值左半区减小/切换，右半区增大/切换；点击设置行只移动光标。
- 单击确认框按钮执行；双击登录方式或选择器条目确认。点击弹窗外部关闭弹窗。滚轮可滚动选择、设置项、帮助和活动日志。
- 终端捕获鼠标时，按住 `Shift` 再拖选通常仍可使用终端自身的文本选择（具体取决于终端模拟器）。

### Dashboard

| 按键 | 行为 |
| --- | --- |
| `Enter` / `p` | 启动活跃实例；没有版本时打开下载选择器 |
| `n` | 浏览并下载 Minecraft 版本 |
| `i` | 选择启动加载器，或安装新的加载器 |
| `s` | 搜索 Modrinth |
| `a` | 账号界面 |
| `r` | 刷新本地数据 |

### Versions

| 按键 | 行为 |
| --- | --- |
| `Enter` | 启动选中版本 |
| `v` | 设为活跃实例 |
| `i` | 为选中版本选择/安装加载器 |
| `n` | 下载其他版本 |
| `d` | 删除选中实例（确认后删除目录、存档和模组） |
| `R` | 重命名选中实例 |
| `c` | 复制选中实例（包含存档与模组） |
| `e` | 导出为 Modrinth `.mrpack`（输入目标路径） |
| `r` | 刷新 |

### Mods

| 按键 | 行为 |
| --- | --- |
| `Space` | 启用 / 禁用选中模组 |
| `x` | 卸载（确认） |
| `s` / `/` | 搜索 Modrinth |
| `v` | 切换活跃 Minecraft 版本 |
| `u` | 检查 Modrinth 更新（后台任务） |
| `U` | 应用全部可用更新（确认后下载并替换） |
| `r` | 刷新 |

### Content

| 按键 | 行为 |
| --- | --- |
| `t` / `[` / `]` | 在存档和资源包之间切换 |
| `↑` `↓` / `PgUp` `PgDn` | 移动选择 |
| `space` / `Enter` | 资源包：启用 / 禁用；存档：备份 |
| `b` | 备份所选存档 |
| `i` | 导入存档或资源包（输入路径） |
| `x` | 删除所选内容（确认） |
| `o` | 打开存档 / 资源包目录 |
| `v` | 切换活跃实例 |
| `r` | 刷新 |

### Accounts

| 按键 | 行为 |
| --- | --- |
| `a` | 添加离线账号 |
| `m` | Microsoft 登录（浏览器 / 设备码） |
| `d` | 设为默认账号 |
| `x` | 登出并删除本地账号 |

### Settings

| 按键 | 行为 |
| --- | --- |
| `←` / `→` | 调整当前项（语言项会切换 English / 中文，GC 项循环预设） |
| `Enter` | 打开 Java 运行时选择器 / 编辑 JVM 参数 / 切换开关 / 切换界面语言 |
| `s` | 立即保存（修改通常会自动保存） |
| `R` | 恢复默认值（保留活跃版本） |

Java 行会列出 `JAVA_HOME`、`PATH`、系统扫描和 `<game_dir>/java` 下的运行时，按当前实例的 Java 要求标注推荐项；选择器最后一项可以手动输入路径。JVM 参数按 shell 风格拆分，留空表示不追加。

### About

| 按键 | 行为 |
| --- | --- |
| `g` | 在浏览器中打开 GitHub 仓库 |
| `b` | 在浏览器中打开 Codeberg 仓库 |

### Activity

| 按键 | 行为 |
| --- | --- |
| `↑` `↓` / `PgUp` `PgDn` | 滚动 |
| `g` | 跳到最早一条 |
| `G` | 回到实时跟随 |
| `f` | 暂停 / 恢复跟随 |
| `c` | 清空（确认） |

## 配置

界面语言默认为 English。打开 Settings，第一项「Language / 界面语言」即可切换到中文，立即生效并写入 `language`。

配置保存在游戏目录（`--dir`）下的 `launcher_tui.json`，由 TUI 自动维护，不含任何令牌：

```json
{
  "ram_mb": 4096,
  "width": 1280,
  "height": 720,
  "threads": 16,
  "java": "",
  "language": "en",
  "active_loader": "fabric",
  "source": "auto",
  "jvm_args": "",
  "gc": "auto",
  "reduced_motion": false,
  "splash": true,
  "active_version": "1.21.4"
}
```

- `ram_mb`：128–65536，512 MB 步进（与 CLI `--ram` 一致）。
- `width` / `height`：0 表示交给 Minecraft 决定；其余会限制在安全范围。
- `threads`：1–64，默认 16；下载器复用 HTTP/TLS 连接。资源文件使用 `threads × 2` 的并发（至少 16、最多 64；镜像上限 24）。
- `language`：`en`（默认）或 `zh`；在 Settings 第一项切换后立即生效并保存。命令行与日志中的底层工具输出仍保持英文。
- `jvm_args`：附加 JVM 参数，按 shell 风格拆分后作为独立 argv 传入；空字符串表示不追加。
- `gc`：`auto` / `g1` / `zgc` / `parallel` / `serial` / `none`；与 CLI `--gc` 一致，ZGC 的分代模式只在 Java 21+ 启用。
- `active_loader`：`fabric` / `forge` / `neoforge`，或用 `null` 表示自动选择第一个已安装加载器；未安装的加载器会先要求安装。
- `source`：`auto` / `official` / `bmcl`。`auto` 会为资源文件同时测速官方与 BMCLAPI 并整批使用更快的一方（失败文件回退另一源），库与元数据仍官方优先；`official` 完全禁用镜像。
- `java`：空表示自动检测。
- 首次打开 TUI 时，命令行的 `--ram`、`--threads`、`--java`、`--width`、`--height`、`--official`、`--bmcl` 会覆盖配置文件；`-v` 会选择已安装的版本作为活跃实例。之后以文件为准。
- 文件损坏时界面会提示并采用默认值；建议手动修复或删除后重新配置。

## 下载性能

资源（assets）是大量小文件，延迟和握手成本远高于字节数。CLI/TUI 共用以下优化：

- 进程级 HTTP agent 复用 TCP/TLS 连接，每个主机最多保留 128 条空闲连接，避免每个资源重新握手。
- `auto` 源模式先用 2 个小型资源对象**并行探测**官方与 BMCLAPI，整批走更快的一方；单个文件失败仍会回退另一源，最后再对失败文件单独重试一轮（TLS 抖动不再导致整批失败）。
- 资源并发为 `threads × 2`（至少 16、最多 64）；BMCLAPI 对高并发握手敏感，镜像上限固定 24。默认 16 线程时资源池为 32。
- 空闲 2 秒后冻结渐变、Logo 与粒子时钟，只以约 8 FPS 重绘时钟；有任务、Toast、Splash 或最近输入时立即恢复 30 FPS（`reduced_motion` 固定 10 FPS）。120×40 伪终端实测空闲流量从约 118 KiB/s 降到 **0.87 KiB/s**。
- 资源先写同目录临时文件，校验 SHA-1 后原子改名；不再对每个小文件执行 `fsync`，避免磁盘屏障把并发下载串行化。
- 缺失文件快速跳过哈希探测；已存在文件仍会校验，损坏缓存仍会重新下载。

思路参考了 PCL2/HMCL 的高并发、多源调度和连接复用，但没有复制它们的代码。原子发布、SHA-1 校验和进度/取消语义保持不变。

延伸阅读：PCL2 下载引擎分析 <https://cloud.tencent.cn/developer/article/2681906>；HMCL 多源调度与断点续传分析 <https://blog.gitcode.com/e84f692185c35e19fa0419450bbca3ab.html>；HMCL `DownloadProviders` <https://github.com/HMCL-dev/HMCL/blob/main/HMCL/src/main/java/org/jackhuang/hmcl/setting/DownloadProviders.java>。

## 安全与边界

- TUI 界面不读取、不显示 Microsoft 访问/刷新令牌。账号列表只使用账号元数据；只有登录与启动时 CLI 核心才访问钥匙串，并会临时离开备用屏幕，让系统钥匙串回退确认、设备码和游戏控制台正常显示。
- 长任务在后台线程或子进程中运行。点击 `c` 或退出时，下载会协作式取消；Forge/NeoForge/模组安装的 CLI 子进程会连同其后代进程一起终止（Unix 进程组 / Windows `taskkill /T`）。
- Forge-family 安装器仍会执行第三方代码；TUI 不改变“只使用可信项目”的建议。
- 模组安装使用当前活跃版本和已检测到的加载器；TUI 会拒绝在没有加载器的实例上安装模组。默认安装与当前版本/加载器兼容的最新模组版本；需要固定 `--mod-version` 时继续使用 CLI。依赖不会自动确认安装，结果会显示在活动日志中。
- 加载器安装使用推荐/最新版本；需要固定 `--loader-version` 时继续使用 CLI。
- TUI 不是并发安装器：同一时间只运行一个重量级任务；搜索、版本元数据和更新检查属于轻量请求，可以并行。
- 同一 Minecraft 版本下的多个加载器共享目录，切换前仍应备份存档。
- Versions 页的 `d` 会在确认后删除实例目录（含存档和模组），`R`/`c` 会改写版本 JSON 与加载器 profile 的键；需要保留存档时请用 CLI `remove-version --keep-saves`。
- 导出整合包会对已安装 JAR 做 SHA-1 查询；Modrinth 不可达时全部模组会作为 overrides 打包，并在活动日志中提示。导入会安全解压 overrides（拒绝路径穿越），模组文件按 SHA-1 校验后原子写入。
- 启动失败后 TUI 会读取最新崩溃报告/日志，把匹配到的诊断写入活动日志；它只做启发式分析，不修改任何文件。
- Content 页的删除操作都有确认框；CLI 对应命令要求 `--yes`。备份/导入在后台任务中运行，可被 `c` 取消。
- 导入存档会拒绝没有 `level.dat` 的压缩包，重名时自动追加后缀；资源包导入会校验根目录 `pack.mcmeta`。`options.txt` 只重写 `resourcePacks` / `incompatibleResourcePacks` 行，其他设置保持不变。
- 资源包启停写入 `options.txt`，在下次启动游戏时生效；TUI 不会重命名资源包文件。

## 实现与验证

- `src/tui/app.rs`：状态、输入分发、本地数据刷新、后台事件归并。
- `src/tui/jobs.rs`：线程/子进程任务、进度协议、取消与进程树清理，以及更新检查/批量更新/整合包导出、存档备份/导入与资源包导入任务。
- `src/tui/ui/`：布局、各界面、弹窗、Toast、粒子、Splash。
- `src/tui/theme.rs`：真彩色主题、渐变、缓动、进度条。
- `src/progress.rs`：CLI 与 TUI 共用的下载进度总线，以及跨进程的 `@@MC_TUI@@` JSON 进度行。
- `src/instance.rs` / `src/update.rs` / `src/diagnose.rs` / `src/content.rs`：实例生命周期与 .mrpack、Modrinth 更新、崩溃诊断、存档与资源包；TUI 只负责展示与触发。
- 自动测试使用 `TestBackend` 在 120×40、100×32、80×24、60×20、52×16 和过小尺寸下渲染全部界面、弹窗、任务状态、搜索、Splash、Toast 与粒子；同时验证中英文切换、鼠标热区点击/滚轮和弹窗按钮，以及 Java 选择器、JVM/GC 设置、更新标记和实例重命名/删除。
- 开发验证中使用伪终端冒烟测试检查进入/离开备用屏幕、括号粘贴与鼠标模式开关、按键退出和退出码；本轮还实际驱动了 Versions 重命名对话框和 Java 选择器。
- `hot-rs-launcher tui` 在非 TTY 下会明确失败，不会阻塞 CI。

## 为什么还是 CLI-only

TUI 没有引入 GUI 工具包、窗口系统或后台服务；它就是 `hot-rs-launcher` 的一个动作。它复用同一份 `--dir`、下载源、账号安全存储、加载器安装器和模组管理代码，因此任何自动化脚本、CI 或服务器都可以继续只用原来的 CLI 命令。
