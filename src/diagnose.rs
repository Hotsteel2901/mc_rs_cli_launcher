//! Crash report and log diagnosis: turn an opaque stack trace into hints.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warn,
    Error,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "信息",
            Severity::Warn => "警告",
            Severity::Error => "错误",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    pub title: String,
    pub title_en: String,
    pub detail: String,
    pub hint: String,
    pub hint_en: String,
}

#[derive(Clone, Debug, Default)]
pub struct Diagnosis {
    pub source: String,
    pub findings: Vec<Finding>,
    /// The most useful lines of the report, for the full-screen log view.
    pub excerpt: Vec<String>,
}

impl Finding {
    pub fn title_localized(&self, chinese: bool) -> &str {
        if chinese || self.title_en.is_empty() {
            &self.title
        } else {
            &self.title_en
        }
    }

    pub fn hint_localized(&self, chinese: bool) -> &str {
        if chinese || self.hint_en.is_empty() {
            &self.hint
        } else {
            &self.hint_en
        }
    }
}

const RULES: &[(&[&str], Severity, &str, &str)] = &[
    (
        &[
            "java.lang.OutOfMemoryError",
            "insufficient memory for the Java Runtime Environment",
            "Java heap space",
            "GC overhead limit exceeded",
        ],
        Severity::Error,
        "内存不足 (OutOfMemoryError)",
        "调高内存（play --ram 6G）；大型整合包建议 6-8 GB，并减少同时加载的模组。",
    ),
    (
        &[
            "UnsupportedClassVersionError",
            "has been compiled by a more recent version of the Java Runtime",
            "requires Java 17 or higher",
            "requires Java 21 or higher",
            "requires Java 8 or higher",
        ],
        Severity::Error,
        "Java 版本不匹配",
        "用 list-java 查看本机 JDK，play --java <路径> 指定，或 install-java --java-major 21。",
    ),
    (
        &[
            "ModResolutionException",
            "Incompatible mod set",
            "Mod resolution failed",
            "Duplicate mods",
            "duplicate mods found",
        ],
        Severity::Error,
        "模组依赖冲突",
        "查看崩溃报告里的 requires 列表，移除冲突模组或补齐缺失依赖。",
    ),
    (
        &[
            "Mixin apply failed",
            "MixinTransformerError",
            "org.spongepowered.asm.mixin",
        ],
        Severity::Error,
        "Mixin 注入失败",
        "通常是模组之间或模组与加载器版本不兼容；更新或移除最近添加的模组。",
    ),
    (
        &[
            "NoSuchMethodError",
            "NoSuchFieldError",
            "ClassNotFoundException",
            "NoClassDefFoundError",
        ],
        Severity::Error,
        "类或方法缺失（模组不兼容或缺前置）",
        "模组版本与 Minecraft/加载器不匹配；确认模组支持当前版本，并安装必需前置。",
    ),
    (
        &[
            "GLFW error",
            "Failed to create window",
            "Pixel format not accelerated",
            "Could not initialize GLX",
            "Failed to set video mode",
            "No OpenGL context",
        ],
        Severity::Error,
        "图形/显示环境初始化失败",
        "更新显卡驱动；远程或无头环境需要 X11/Wayland + OpenGL；可关闭光影或降低分辨率。",
    ),
    (
        &[
            "Invalid session",
            "Failed to verify username",
            "authentication servers are down",
            "user not authenticated",
            "Invalid token",
        ],
        Severity::Warn,
        "登录会话无效",
        "重新运行 login；要求正版验证的服务器不能用离线账号进入。",
    ),
    (
        &[
            "UnknownHostException",
            "Connection refused",
            "Connection timed out",
            "Failed to download",
        ],
        Severity::Warn,
        "网络连接失败",
        "检查网络或代理，或使用 --bmcl 切换镜像；离线启动需要资源已提前下载。",
    ),
    (
        &["No space left on device", "not enough space on the disk"],
        Severity::Error,
        "磁盘空间不足",
        "清理磁盘，或把 --dir 指向空间更大的位置。",
    ),
    (
        &[
            "AccessDeniedException",
            "Permission denied",
            "Read-only file system",
        ],
        Severity::Error,
        "文件权限不足",
        "检查游戏目录权限，必要时用当前用户重新下载。",
    ),
    (
        &[
            "Could not find or load main class",
            "Could not find the main class",
        ],
        Severity::Error,
        "安装不完整（找不到主类）",
        "重新执行 download -v <版本>，或重新安装加载器。",
    ),
    (
        &["java.lang.StackOverflowError"],
        Severity::Error,
        "栈溢出",
        "通常是模组递归或死循环；更新相关模组或移除最近添加的模组。",
    ),
    (
        &[
            "EXCEPTION_ACCESS_VIOLATION",
            "Exit Code: -1073741819",
            "0xC0000005",
        ],
        Severity::Error,
        "原生崩溃（访问冲突）",
        "常见于显卡驱动或原生模组；更新驱动，移除最近添加的模组。",
    ),
    (
        &["EXCEPTION_STACK_OVERFLOW"],
        Severity::Error,
        "原生栈溢出",
        "减少模组或更新相关原生库。",
    ),
    (
        &["NullPointerException", "ArrayIndexOutOfBoundsException"],
        Severity::Warn,
        "模组抛出未捕获异常",
        "在堆栈里找到第一个非 Minecraft 的包名，定位出问题的模组。",
    ),
];

/// English equivalents for RULES, in the same order.
const ENGLISH: &[(&str, &str)] = &[
    (
        "Out of memory (OutOfMemoryError)",
        "Raise the heap (play --ram 6G); large modpacks usually need 6-8 GB.",
    ),
    (
        "Java version mismatch",
        "Run list-java, pass play --java <path>, or run install-java --java-major 21.",
    ),
    (
        "Mod dependency conflict",
        "Read the requires list in the crash report, then remove or install the mods it names.",
    ),
    (
        "Mixin injection failed",
        "Usually a mod/loader version mismatch; update or remove recently added mods.",
    ),
    (
        "Missing class or method (mod mismatch)",
        "The mod does not match this Minecraft/loader version; check its dependencies.",
    ),
    (
        "Graphics/display initialisation failed",
        "Update the GPU driver; remote sessions need X11/Wayland with OpenGL.",
    ),
    (
        "Invalid login session",
        "Run login again; online-mode servers reject offline accounts.",
    ),
    (
        "Network connection failed",
        "Check the network or proxy, or use --bmcl; offline launches need cached assets.",
    ),
    (
        "Not enough disk space",
        "Free space or point --dir at a larger disk.",
    ),
    (
        "Insufficient file permissions",
        "Fix ownership or permissions of the game directory.",
    ),
    (
        "Incomplete installation (main class missing)",
        "Re-run download -v <version> or reinstall the loader.",
    ),
    (
        "Stack overflow",
        "Usually a mod recursion bug; update or remove the offending mod.",
    ),
    (
        "Native crash (access violation)",
        "Often the GPU driver or a native mod; update drivers and remove recent mods.",
    ),
    (
        "Native stack overflow",
        "Reduce mods or update native libraries.",
    ),
    (
        "Uncaught exception in a mod",
        "Find the first non-Minecraft package in the stack trace.",
    ),
];

fn clip(value: &str, max: usize) -> String {
    let value = value.trim();
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut result: String = value.chars().take(max.saturating_sub(1)).collect();
    result.push('…');
    result
}

fn extract_mod_name(line: &str) -> Option<String> {
    let re = regex::Regex::new(r"(?:Mod|mod)\s+'([^']+)'\s+(?:requires|is missing|failed)").ok()?;
    re.captures(line)
        .and_then(|caps| caps.get(1).map(|name| name.as_str().to_string()))
        .or_else(|| {
            let re = regex::Regex::new(r"^\s*-+\s*(\S+)\s*$").ok()?;
            re.captures(line)
                .and_then(|caps| caps.get(1).map(|name| name.as_str().to_string()))
        })
}

/// Scan a crash report or log and explain the most likely cause.
pub fn analyze_text(text: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    let lines: Vec<&str> = text.lines().collect();

    if let Some(description) = header_value(text, "Description") {
        findings.push(Finding {
            severity: Severity::Info,
            title: "崩溃描述".into(),
            title_en: "Crash description".into(),
            detail: clip(&description, 200),
            hint: "描述里的关键词通常能定位到具体模组或阶段（初始化、渲染、连接服务器等）。".into(),
            hint_en: "The description usually names the failing stage or mod.".into(),
        });
    }
    if let Some(java) = header_value(text, "Java Version") {
        findings.push(Finding {
            severity: Severity::Info,
            title: "崩溃时的 Java".into(),
            title_en: "Java at crash".into(),
            detail: clip(&java, 160),
            hint: "老版本 Minecraft 需要 Java 8/17；新版本需要 21。可用 list-java 核对。".into(),
            hint_en: "Older Minecraft needs Java 8/17; newer needs 21. Check with list-java."
                .into(),
        });
    }

    for (index, (needles, severity, title, hint)) in RULES.iter().enumerate() {
        let hit = lines
            .iter()
            .find(|line| needles.iter().any(|needle| line.contains(needle)));
        let Some(line) = hit else {
            continue;
        };
        let mut detail = clip(line, 240);
        if let Some(module) = extract_mod_name(line) {
            detail = format!("{detail}  →  相关模组: {module}");
        }
        let (title_en, hint_en) = ENGLISH.get(index).copied().unwrap_or(("", ""));
        findings.push(Finding {
            severity: *severity,
            title: (*title).to_string(),
            title_en: title_en.to_string(),
            detail,
            hint: (*hint).to_string(),
            hint_en: hint_en.to_string(),
        });
    }

    // The classic Forge "missing dependency" block names the culprit after a
    // line with "requires" and before the closing dashes.
    if findings
        .iter()
        .all(|finding| finding.title != "模组依赖冲突")
    {
        if let Some(line) = lines.iter().find(|line| {
            line.contains("requires") && (line.contains("mod") || line.contains("Mod"))
        }) {
            findings.push(Finding {
                severity: Severity::Warn,
                title: "疑似模组依赖问题".into(),
                title_en: "Possible mod dependency problem".into(),
                detail: clip(line, 240),
                hint: "补齐前置模组，或确认模组版本与当前 Minecraft 版本一致。".into(),
                hint_en: "Install missing dependencies or match the mod to this Minecraft version."
                    .into(),
            });
        }
    }

    findings
}

fn header_value(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    text.lines()
        .find_map(|line| {
            line.strip_prefix(&prefix)
                .map(|value| value.trim().to_string())
        })
        .filter(|value| !value.is_empty())
}

/// Important lines for the log view: stack trace heads and error lines.
fn excerpt(text: &str, limit: usize) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains("Exception")
            || trimmed.contains("Error")
            || trimmed.starts_with("Caused by:")
            || trimmed.starts_with("at ")
        {
            result.push(clip(trimmed, 200));
            if result.len() >= limit {
                break;
            }
        }
    }
    result.reverse();
    result
}

fn newest_file(dir: &Path, extension: &str) -> Option<(PathBuf, SystemTime)> {
    let mut best: Option<(PathBuf, SystemTime)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(extension) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(_, current)| modified > *current) {
            best = Some((path, modified));
        }
    }
    best
}

/// Find and analyze the newest crash report (or latest.log) of an instance.
pub fn analyze_instance(game_dir: &Path, version: &str) -> Option<Diagnosis> {
    let instance = game_dir.join("versions").join(version);
    let crash = newest_file(&instance.join("crash-reports"), ".txt")
        .or_else(|| newest_file(&game_dir.join("crash-reports"), ".txt"));
    let (path, kind) = match crash {
        Some((path, _)) => (path, "崩溃报告"),
        None => {
            let log = instance.join("logs").join("latest.log");
            if !log.is_file() {
                return None;
            }
            (log, "最新日志")
        }
    };
    let bytes = std::fs::read(&path).ok()?;
    // Logs can be large; keep the tail where the failure is.
    let text = String::from_utf8_lossy(&bytes);
    let text = if text.len() > 512 * 1024 {
        let start = text.len() - 512 * 1024;
        let start = text
            .char_indices()
            .map(|(index, _)| index)
            .find(|index| *index >= start)
            .unwrap_or(0);
        text[start..].to_string()
    } else {
        text.to_string()
    };
    Some(Diagnosis {
        source: format!("{kind}: {}", path.display()),
        findings: analyze_text(&text),
        excerpt: excerpt(&text, 12),
    })
}

/// Format a diagnosis for the CLI.
pub fn render(diagnosis: &Diagnosis) -> String {
    let mut out = String::new();
    out.push_str(&format!("  来源: {}\n\n", diagnosis.source));
    if diagnosis.findings.is_empty() {
        out.push_str("  未发现已知的崩溃模式。\n");
        out.push_str("  建议：查看完整日志，或用 --verbose 重新启动。\n");
        return out;
    }
    for finding in &diagnosis.findings {
        out.push_str(&format!(
            "  [{}] {}\n",
            finding.severity.label(),
            finding.title
        ));
        if !finding.detail.is_empty() {
            out.push_str(&format!("      {}\n", finding.detail));
        }
        if !finding.hint.is_empty() {
            out.push_str(&format!("      → {}\n", finding.hint));
        }
        out.push('\n');
    }
    if !diagnosis.excerpt.is_empty() {
        out.push_str("  关键日志:\n");
        for line in &diagnosis.excerpt {
            out.push_str(&format!("    {line}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_common_failure_modes() {
        let oom = analyze_text(
            "Exception in thread \"main\" java.lang.OutOfMemoryError: Java heap space\n",
        );
        assert!(oom.iter().any(|f| f.title.contains("内存不足")));

        let java = analyze_text("java.lang.UnsupportedClassVersionError: net/minecraft/client/Main has been compiled by a more recent version of the Java Runtime");
        assert!(java.iter().any(|f| f.title.contains("Java 版本")));

        let mixin = analyze_text(
            "org.spongepowered.asm.mixin.throwables.MixinTransformerError: An unexpected critical error was encountered\n",
        );
        assert!(mixin.iter().any(|f| f.title.contains("Mixin")));

        let clean = analyze_text("Starting Minecraft 1.21.4\nLoading 12 mods\n");
        assert!(clean.is_empty());
    }

    #[test]
    fn extracts_report_headers_and_mod_names() {
        let report = "---- Minecraft Crash Report ----\nDescription: Initializing game\nJava Version: 17.0.8, Eclipse Adoptium\nMod 'sodium' requires version 0.6.0 or later of 'fabric-api'\n";
        let findings = analyze_text(report);
        assert!(findings
            .iter()
            .any(|f| f.title == "崩溃描述" && f.detail == "Initializing game"));
        assert!(findings.iter().any(|f| f.title == "崩溃时的 Java"));
        let dep = findings
            .iter()
            .find(|f| f.title == "疑似模组依赖问题")
            .expect("dependency finding");
        assert!(dep.detail.contains("sodium"));
        assert_eq!(
            header_value("Java Version: 21", "Java Version").unwrap(),
            "21"
        );
    }

    #[test]
    fn every_rule_has_an_english_translation() {
        assert_eq!(RULES.len(), ENGLISH.len());
        let findings = analyze_text("java.lang.OutOfMemoryError: Java heap space");
        let finding = findings.first().expect("finding");
        assert!(finding.title_localized(true).contains("内存"));
        assert!(finding
            .title_localized(false)
            .to_ascii_lowercase()
            .contains("memory"));
        assert!(finding.hint_localized(false).contains("--ram"));
    }

    #[test]
    fn finds_and_analyzes_the_newest_crash_report() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let reports = root.join("versions").join("1.21.4").join("crash-reports");
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(
            reports.join("crash-2024-01-01_10.00.00-client.txt"),
            "Description: Rendering overlay\njava.lang.OutOfMemoryError: Java heap space\n",
        )
        .unwrap();
        let diagnosis = analyze_instance(root, "1.21.4").unwrap();
        assert!(diagnosis.source.contains("crash-2024-01-01"));
        assert!(diagnosis
            .findings
            .iter()
            .any(|f| f.title.contains("内存不足")));
        assert!(!diagnosis.excerpt.is_empty());
        assert!(diagnosis
            .excerpt
            .iter()
            .any(|line| line.contains("OutOfMemory")));

        std::fs::create_dir_all(root.join("versions").join("no-logs")).unwrap();
        assert!(analyze_instance(root, "no-logs").is_none());
        assert!(render(&diagnosis).contains("内存不足"));
    }
}
