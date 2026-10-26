# 验证记录

## 2026-10-26 · Hot-RS-Launcher 2.8.0（本地 Linux）

本轮是加固与性能专项：安全解压、校验缓存、维护命令、代理支持和一批 TUI 修复。

| 验证项目 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| 单元测试 | 147 / 147（另 1 个 `#[ignore]` 手动预览） |
| CLI 集成测试 | 17 / 17 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 |
| `cargo build --locked --release` | 通过 |
| 手动 CLI 冒烟 | `doctor --json`（离线模式）、`prune --json` / `--yes`（保留被引用库与 profile JSON、删除无用库与资源）、`repair --json`（缺 client jar 时报告 FAIL）、`list-installed/list-mods/list-backups --json`、`completions fish` |
| 真实网络（1.20.6，本机 SSD） | 首次 `download`（含 3921 个资源对象，727 MB）186 s；重复执行 0.06 s（校验缓存命中，跳过镜像探测）；`MC_LAUNCHER_NO_VERIFY_CACHE=1` 时 1.46 s（全量重算 SHA-1） |

本轮改动：

- **安全解压**：`.mrpack`、备份恢复、世界导入统一走 `util::extract_zip_entries`，限制条目数（10 万）、单文件（512 MiB）与总展开量（16 GiB），拒绝符号链接、绝对路径、盘符路径和 `..`，流式写入而非 `read_to_end`。世界导入先解压到 `saves/.import-*` 暂存目录，成功后才改名为可见世界，失败自动清理。
- **集成包导入**：失败时用 `PartialDir` 删除半成品实例；`{id}.json` 与 `instance.json` 改为所有文件下载完成后才写入，避免出现「已安装但缺模组」的实例。
- **备份原子性**：`backup`、`backup-save` 与 `.mrpack` 导出改为同目录临时文件 + `persist`，同秒重名自动追加 `-2`；失败不会截断已有归档。
- **校验缓存**：新增 `.launcher_verify.json`，按「路径 + 大小 + 修改时间」记录已验证摘要；重复启动不再重算数万个资源对象的 SHA-1。缓存只跳过哈希，下载仍校验后原子替换；`MC_LAUNCHER_NO_VERIFY_CACHE=1` 可关闭。Java 主版本探测结果也写入同一缓存，第二次启动不再 spawn `java -version`。
- **维护命令**：`doctor`（平台/权限/账号/实例/Java/网络/代理/孤儿存档，`--json`）、`prune`（默认预览，`--yes` 删除未引用库与资源对象，元数据不可解析时拒绝执行）、`repair`（`--deep` 重算 SHA-1，`--yes` 幂等补下）、`completions`（bash/zsh/fish/powershell/elvish）。
- **代理**：`--proxy` 或 `HTTPS_PROXY`/`HTTP_PROXY`/`ALL_PROXY`，拒绝 SOCKS（本构建未启用该特性）。
- **下载健壮性**：429/503 尊重 `Retry-After`，重试加入抖动避免并发同步重试；模组文件同名但内容不同时拒绝静默覆盖；`update` 的文件名在计划阶段即校验。
- **依赖判定**：改为读取 jar 内 `fabric.mod.json` / `mods.toml` 的 modId，文件名回退只接受版本样式后缀；`sodium-extra.jar` 不再被误判为 `sodium`。`incompatible` 依赖不再作为「可选推荐」提示安装。原版实例（未检测到加载器）拒绝安装模组，避免装进其他加载器的 jar。
- **TUI**：加载器安装任务补上 `set_job`（下载进度不再恒为 0）；退出与 panic 路径补 `DisableMouseCapture`；`resume` 失败回滚 raw mode；退出时恢复原 panic hook；失败任务不再让界面永久保持 30 FPS；光影详情页的加载器探测改为 1.5 秒缓存并只扫描一次 mods/。
- **实例与存储**：删除/重命名基础版本时保留仍被其他实例继承的加载器 profile；实例大小改为单次迭代遍历并缓存 5 秒；`mods_dir` 遇到非法实例 id 不再 `process::exit`，避免 TUI 线程直接终止进程。

未覆盖：真实 zip bomb 的端到端下载测试（仅构造条目验证限制）、Windows/macOS 的代理与文件锁差异、真实大型整合包导入的耗时基线。

---

## 2026-10-24 · Hot-RS-Launcher 2.7.0（本地 Linux）

本轮新增 Modrinth 浏览下载层（模组 / 资源包 / 光影包）、光影加载器识别与提示、优化模组推荐安装，以及依赖自动安装。

| 验证项目 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| 单元测试 | 139 / 139（另 1 个 `#[ignore]` 手动预览） |
| CLI 集成测试 | 13 / 13 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 |
| `cargo build --locked --release` | 通过 |
| 伪终端冒烟（120×40） | `3` → `s` 打开模组浏览；`]`/`c`/`b` 翻页、切分类、回浏览；`4` → `t` → `d` 分别浏览资源包和光影；光影结果显示 `Iris + OptiFine`；退出码 0 |
| 真实网络（CLI） | 浏览/搜索模组、资源包、光影；下载资源包并启用；下载 BSL 光影并启用；`--install-loader` 自动安装 Iris + Sodium；`optimize-mods --apply --yes` 批量安装；`install-mod --yes` 自动安装 Iris 的 Sodium 依赖 |

新增自动测试覆盖：

- 光影：options.txt / optionsof.txt 的 `shaderPack` 读写、启用/禁用/删除、加载器元数据记录、Iris/OptiFine 安装探测、zip/文件夹导入校验。
- 浏览：默认浏览状态、翻页、optimization 分类切换、浏览重置、结果解析（加载器标签、已安装标记）、TUI 渲染。
- 光影加载器提示：支持 Iris 时提示安装 Iris；仅 OptiFine 时显示手动安装提示。
- 优化模组：快捷键确认、Iris/Oculus 过滤、批量安装路径（真实网络手测）。
- CLI：`list-shaders`/`enable-shader`/`disable-shader`/`import-shader`/`delete-shader` 的离线端到端测试。

未覆盖：真实 Minecraft 客户端读取光影/资源包设置、真实 OptiFine 安装流程、Windows/macOS 文件管理器与终端差异、大量模组同时安装的冲突求解。

---

## 2026-10-24 · Hot-RS-Launcher 2.6.0（本地 Linux）

本轮新增存档与资源包管理（CLI + TUI）。

| 验证项目 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| 单元测试 | 128 / 128（另 1 个 `#[ignore]` 手动预览） |
| CLI 集成测试 | 12 / 12 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 |
| `cargo build --locked --release` | 通过 |
| 伪终端冒烟（120×40） | `4` 进入内容页；`t` 切换资源包；空格禁用后 options.txt 更新；`t` 返回存档；`x` 打开删除确认并用 `n` 取消；退出码 0 |
| 手动 CLI 冒烟 | list-saves（NBT 名称/模式/极限标记）、list-resourcepacks、backup-save、delete-save --yes、import-save、enable/disable-resourcepack、import/delete-resourcepack、`--json` 输出 |

新增自动测试覆盖：

- level.dat：gzip/原始 NBT 解析（LevelName、Version.Name、GameType、hardcore），损坏时回退。
- 存档：忽略没有 level.dat 的目录；大小/图标/时间；备份 ZIP 往返；原版世界 ZIP（根 level.dat）；重名自动追加后缀；拒绝非世界压缩包；删除与路径穿越拒绝。
- 资源包：options.txt 读写保留其他键；启用置顶；incompatible 清理；zip/文件夹导入校验；删除同步清理引用；pack.mcmeta 描述与 pack_format。
- TUI：内容页 tab 切换、资源包启停写回 options.txt、删除确认、不同终端尺寸渲染、鼠标点击 tab。
- CLI：上述命令的离线端到端测试与 JSON 输出。

未覆盖：真实 Minecraft 客户端读取 options.txt/存档的兼容性、Windows/macOS 文件管理器打开目录、数 GB 大型世界备份耗时。

---

## 2026-10-24 · Hot-RS-Launcher 2.5.0（本地 Linux）

本轮新增：Java 运行时发现/安装、实例重命名/复制/删除、备份恢复、Modrinth .mrpack 导入导出、模组更新检查与应用、崩溃诊断、启动参数（GC / JVM 参数 / 直连 / 全屏 / dry-run），以及 TUI 对应界面与快捷键。

| 验证项目 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| 单元测试 | 117 / 117（另 1 个 `#[ignore]` 手动预览） |
| CLI 集成测试 | 11 / 11 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 |
| `cargo build --locked --release` | 通过 |
| 伪终端冒烟（120×40） | 启动/退出码 0；`2` → `R` 重命名后磁盘目录与 JSON 同步；`5` → Java 选择器显示 Auto-detect 与推荐项；终端恢复 |
| 手动 CLI 冒烟 | `list-java`、`diagnose`、`rename/copy/remove`、`export/import` .mrpack、`backup/list-backups`、`play --dry-run`（不下载、不刷新令牌）、管道到 `head` 时不再 panic |

新增自动测试覆盖：

- Java：`release` 解析、`java -version` 回退、managed runtime 扫描、Mojang 组件选择、推荐项优先级。
- 实例：删除（保留存档）、重命名/复制的 JSON/JAR/profile 重键、marker 基准版本、备份恢复、.mrpack 往返（离线）。
- 更新：同版本/旧版本不产生更新、禁用模组保持禁用、文件名映射。
- 诊断：OOM / Java 版本 / Mixin / 依赖 / GLFW 等规则、崩溃报告选择、英文本地化。
- 启动参数：GC 预设（含 ZGC 分代条件）、服务器地址校验（含 IPv6 与注入拒绝）、shell 引号显示、`--jvm-args` 拆分、dry-run 命令输出。
- TUI：Java 选择器 / JVM / GC 设置、版本重命名/删除、更新标记渲染、Dashboard 更新提示。

未覆盖：真实 Microsoft 授权、真实 Modrinth 更新下载与真实 .mrpack 包、Windows/macOS 终端差异、真实 Minecraft 图形启动。

---

## 2026-10-18 · Hot-RS-Launcher 2.4.0 复验（本地 Linux）

本轮包含改名、TUI「关于」页、CLI `--about`、资源下载调度重写、账号/Java 路径/空闲渲染等修复。以下结果来自本地工作树（Linux x86_64，8 核，Rust 1.99），尚未包含推送后的三平台 CI 结果。

| 验证项目 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| 单元测试 | 89 / 89（另 1 个 `#[ignore]` 手动预览） |
| CLI 集成测试 | 7 / 7 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 |
| `cargo build --locked --release` | 通过（stripped 5.2 MB） |
| TUI 伪终端冒烟（120×40） | 备用屏进入/恢复、鼠标模式、`7` 打开关于页、`q` 退出码 0 |
| 真实网络 | Modrinth 版本/搜索；Mojang 下载 1.0 与 1.21.4（客户端、70 个库、4039 个资源，SHA-1 全部校验） |

### 资源下载基准（Minecraft 1.21.4，4039 个对象 / 413 MB，同一网络）

| 版本与参数 | 资源阶段 | 全流程 |
| --- | --- | --- |
| 2.3.0 `auto`（镜像优先，16 并发） | ≈57 s | 61.6 s |
| 2.4.0 `auto`（先测速→官方，32 并发） | ≈11.4 s | 15.6 s |
| 2.4.0 `--official`（32 并发） | ≈11.0 s | 16.5 s |
| 2.4.0 再次运行（全部缓存命中并校验） | — | 4.0 s |

BMCLAPI 在本机观测到按 IP 限流：首次 16 并发约 57 s 正常完成；连续压测后 64 并发出现 TLS 握手重置（261 s 后失败），16 并发退化到 183 s。因此实现把镜像并发上限固定为 24，并对失败文件追加一轮单独重试。上述镜像数据受限流影响，只能说明需要保守并发，不能代表常态速度。

### 界面空闲流量（TUI，120×40，伪终端）

| 实现 | 空闲 5 s 内输出 |
| --- | --- |
| 2.3.0（30 FPS 持续重绘） | 约 118 KiB/s |
| 仅降到 8 FPS | 约 55 KiB/s |
| 2.4.0（空闲冻结环境动画，8 FPS 重绘时钟） | **0.87 KiB/s** |

### 本轮新增的回归测试

- 无钥匙串时 `logout` 仍清空本地元数据，且删除路径不会创建 `launcher_secrets.key` / `.enc`。
- 游戏目录含 `[` `*` 等 glob 元字符时仍能找到已下载的 Java runtime。
- TUI Java 路径接受 JDK 目录、拒绝不存在/非 Java 文件，并显示探测到的 Java 主版本。
- CLI/TUI 的 RAM 边界统一为 128–65536 MB。
- 关于页包含作者、两个仓库地址与许可证；`--about` 输出同样内容且不创建游戏目录。
- `--threads` 与资源并发、镜像并发上限的边界。

本轮未覆盖：Windows / macOS（等待推送后的 CI）、真实 Microsoft 授权、真实 Minecraft 图形界面启动。

---

日期：2026-10-14（该项目当时名为 `mc-launcher`，下文的命令名与截图保留当时的写法）。

## 最终结果：三平台通过

- 被测代码提交：`6d8cd4889caaee6b00be6cf35ab12683b6f6cbb3`。
- 工作分支：`arena/2bf40672-mc-rs-cli-launcher`，未合并到 `main`。
- [GitHub Actions 完整运行记录](https://github.com/Hotsteel2901/mc_rs_cli_launcher/actions/runs/38543818916)，最终结论 **success**。
- 本报告是在该运行完成后补充的文档，不改变被测代码。

| 验证项目 | Linux | Windows | macOS |
| --- | --- | --- | --- |
| `cargo fmt --all -- --check` | 通过 | 通过 | 通过 |
| 单元测试 | 43 / 43 | 43 / 43 | 43 / 43 |
| CLI 集成测试 | 5 / 5 | 4 / 4 | 5 / 5 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 | 通过 | 通过 |
| `cargo build --locked --release` | 通过 | 通过 | 通过 |

Windows 少一项是因为假 Java shell 脚本启动测试仅在 Unix 上运行，不是运行失败或临时忽略测试。所有测试均无失败。

## 验证过程

沙箱未预装 Rust；通过 npm 的预打包工具链安装后，本地 `cargo test --locked` 仍无法读取 `https://index.crates.io/config.json`（TLS/出网限制）。因此没有把本地完整编译或测试记为通过，而是推送当前工作分支，通过真正的 GitHub 托管 runner 验证。

第一轮验证发现 macOS 临时目录路径别名（`/var` 与 `/private/var`）导致的测试断言问题，以及 Clippy 的多余 let-return 警告；均已修复并重新执行。最后一轮三平台的格式、测试、严格 lint 和 release 构建全部通过。

本地另外执行了：Rustfmt WASM 格式化/语法解析、`git diff --check`，以及 CI 诊断脚本成功/失败退出码冒烟测试。CI 日志下载域名也受沙箱限制，因此增加了通过 GitHub Checks annotations 读取测试摘要和失败详情的机制，没有降低任何检查标准。

## 自动测试范围

- HTTP：本地 TCP fixture 模拟成功、HTTP 503、截断响应和错误 SHA-1；验证错误状态保留、写盘错误传播、临时文件清理、旧文件保留和摘要缓存复用。
- 镜像策略：官方/镜像/自动选项、完整域名匹配、防止相似域名和 URL 参数误替换。
- 账号：模拟钥匙串，验证令牌不写入 JSON、旧明文迁移、钥匙串不可用时拒绝明文回退、logout、用户名校验、损坏 JSON 拒绝覆盖及 Unix 文件权限。
- 元数据：Maven 坐标和扩展名、路径穿越/Windows 保留名、UUID、操作系统与 features 规则、损坏资源 hash、缓存版本及 latest 别名、原生库更新、加载器库覆盖和 profile 父版本校验。
- 模组：启用/禁用/卸载、唯一匹配、歧义查询不改文件、只读列表不创建目录。
- CLI：参数冲突、RAM/线程/窗口大小边界、版本参数区别、离线账号生命周期、无网络列表、损坏账号文件报错。
- 启动：Unix 上用假 Java 验证相对路径、带空格的目录、classpath/游戏目录的绝对路径、用户名、RAM、窗口尺寸、工作目录和子进程退出码。测试 JAR 是自建 fixture，不包含 Minecraft 游戏文件。

## 尚未做的真实环境测试

不能把上述结果等同于以下项目已经通过：

- 真实 Microsoft → Xbox Live → XSTS → Minecraft 授权及刷新；尤其是应用 ID 的服务端授权要求。
- Linux Secret Service、Windows 凭据管理器、macOS Keychain 的真实交互。
- Mojang、BMCLAPI、Modrinth 实际下载和网络故障恢复。
- Fabric / Forge / NeoForge 的真实安装器与 processors 全流程。
- Minecraft 图形客户端、音频/原生库、存档和模组兼容性。

这些需要可访问对应服务、有正确 Java/图形环境和合法账号的机器。建议发布前使用独立测试目录完成下列人工验收，不要直接使用重要存档：

1. 离线账号下载并启动一个现代正式版，检查窗口尺寸、声音、资源和退出码。
2. 故意截断缓存 JAR，再次下载，确认检测损坏并修复。
3. 在系统钥匙串已解锁时进行 Microsoft 登录、重启启动器、会话刷新、logout；确认 JSON 无令牌，钥匙串条目正确清理。
4. 分别在独立目录安装 Fabric / Forge / NeoForge，启动到主菜单；添加兼容模组后检查依赖与加载器选择。
5. 断网后启动文件齐全的指定版本；检查缺文件/过期认证会明确报错，不假装启动成功。
6. 在真实终端中运行 `mc-launcher tui`，验证调整窗口、切换界面、取消下载/安装、退出后终端恢复；在 Windows Terminal / macOS Terminal 上分别检查真彩色和按键行为。

## TUI 新增验证（2026-10-17，本地 Linux）

本轮新增的 `mc-launcher tui` 在本地 Linux 环境完成：

- `cargo fmt --all -- --check`
- `cargo test --locked --all-targets`：单元测试与 CLI 集成测试全部通过；TUI 渲染测试覆盖 120×40、100×32、80×24、60×20、52×16 及过小终端，覆盖全部界面、搜索、帮助、确认、输入、版本/加载器选择器、登录选择、Splash、Toast、粒子、运行/成功/失败任务状态。
- `cargo clippy --locked --all-targets -- -D warnings`
- `cargo build --locked --release`
- 伪终端（PTY）冒烟：100×32 下启动、跳过 Splash、切换界面、打开/关闭帮助、`q` 退出；确认进入/离开备用屏幕、括号粘贴启用/禁用、退出码 0。
- 非 TTY：`mc-launcher tui` 输出交互式终端错误并立即退出，未创建游戏目录或配置。

尚未完成：Windows/macOS 真实终端的视觉效果与子进程树清理分支、真实 Forge/NeoForge 安装器在 TUI 中的长时间运行与取消、真实 Microsoft 登录在 TUI 挂起/恢复后的钥匙串回退交互。这些仍属于发布前人工验收范围。

## TUI 中英双语与鼠标验证（2026-10-17，本地 Linux）

- `cargo fmt --all -- --check`
- `cargo clippy --locked --all-targets -- -D warnings`
- `cargo test --locked --all-targets`：84 个单元测试 + 6 个 CLI 集成测试通过（另有 1 个手动预览测试默认忽略）。新增用例覆盖 `language` 配置归一化、Settings 第一项即时切换中英文、侧栏/列表/设置值/确认按钮/选择器的鼠标热区、弹窗关闭和滚轮移动。
- `cargo build --locked --release`
- 伪终端（PTY）100×32：启动、点击/按键跳过 Splash、发送鼠标按下/释放与滚轮事件、切换界面、帮助、`q` 退出；确认进入/离开备用屏幕、括号粘贴和鼠标捕获启用/禁用、退出码 0。
- 伪终端（PTY）中文配置：`launcher_tui.json` 写入 `language: "zh"` 后启动并切换到 Settings，确认 Dashboard、Settings、侧栏、状态栏显示中文，宽字符对齐正常，退出码 0。搜索、帮助与确认框的中文渲染另由 `TestBackend` 人工预览核对。
- 中英文界面在 110×34 下人工渲染核对：中文模式面板标题、字段、描述、按钮、帮助、确认框、选择器与状态栏均切换；英文仍是默认并保留原测试断言。宽字符在 `TestBackend` 转储中按单元格显示为间隙，但真实终端显示正常。

尚未完成：Windows Terminal / macOS Terminal / 其他终端模拟器的鼠标协议差异、按住 Shift 文本选择的实际行为，以及真实中文输入法在搜索框中的组合输入。这些仍属于发布前人工验收范围。

## 资源下载性能优化（2.3.0，本地 Linux）

- 全局 HTTP agent、每主机 64 条空闲连接；资源下载源顺序改为 `auto` 下 BMCLAPI 优先、同轮重试回退官方。
- 默认线程 4 → 16（上限 32 → 64），assets 池至少 16；移除逐文件 `fsync`，保留临时文件 + SHA-1 + 原子改名。
- 新增 `asset_source_order_prefers_the_mirror_only_in_auto_mode` 测试；HTTP 下载、校验、取消和 UI 测试全部通过。
- `cargo test --locked --all-targets`：84 个单元测试 + 6 个 CLI 集成测试通过（另有 1 个手动预览测试默认忽略）。
- `cargo clippy --locked --all-targets -- -D warnings`、`cargo build --locked --release`、`cargo fmt --all -- --check` 通过。
- 未在本机对真实 Mojang/BMCLAPI 外网做带宽基线测量；发布前建议在目标网络用 `mc-launcher download -v <版本>` 对比旧版/`--official`/`--bmcl`。

## 仍有的产品边界

同一 Minecraft 版本下多个加载器共享游戏目录；模组依赖处理不是完整冲突求解器；没有跨进程安装锁；旧式不含 `version.json` 的 Forge-family 安装器不支持。文档已明确这些限制，不宣称“所有版本、账号、模组组合都已验证”。
