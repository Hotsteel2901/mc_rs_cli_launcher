# 验证记录

日期：2026-10-14。

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
