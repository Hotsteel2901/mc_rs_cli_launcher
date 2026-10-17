//! Lightweight, dependency-free UI localisation for the TUI.
//!
//! English is the default. Only the full-screen frontend is translated; CLI
//! output remains English so scripts and error messages stay stable.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    English,
    Chinese,
}

impl Lang {
    pub fn from_code(code: &str) -> Self {
        match code.trim().to_ascii_lowercase().as_str() {
            "zh" | "zh-cn" | "zh_cn" | "cn" | "chinese" | "中文" => Self::Chinese,
            _ => Self::English,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Chinese => "zh",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Chinese => "中文",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::English => Self::Chinese,
            Self::Chinese => Self::English,
        }
    }

    pub fn pick<'a>(self, english: &'a str, chinese: &'a str) -> &'a str {
        match self {
            Self::English => english,
            Self::Chinese => chinese,
        }
    }
}

/// Static labels emitted by the job runner are mapped here for the job panel.
/// Unknown strings (installer output, errors from the CLI core) stay as-is.
pub fn localize_job_phase(lang: Lang, label: &str) -> &str {
    if lang == Lang::English {
        return label;
    }
    match label {
        "Preparing" => "准备中",
        "Resolving version metadata" => "解析版本元数据",
        "Fetching client jar" => "下载客户端 JAR",
        "Libraries and natives" => "库与原生文件",
        "Assets skipped" => "已跳过资源",
        "Downloading assets" => "下载资源文件",
        "Fetching Fabric profile" => "获取 Fabric 配置",
        "Saving loader profile" => "保存加载器配置",
        "Finalizing loader profile" => "完成加载器配置",
        "Running launcher worker" => "运行安装器",
        _ => label,
    }
}

/// Common job titles are built by the frontend, but child workers also send
/// their own; this keeps the panel consistent if one arrives late.
pub fn localize_job_result(lang: Lang, message: &str) -> String {
    if lang == Lang::English {
        return message.to_string();
    }
    if let Some(version) = message.strip_prefix("Minecraft ") {
        if let Some(version) = version.strip_suffix(" is ready to play") {
            return format!("Minecraft {version} 已就绪，可以启动");
        }
    }
    if let Some(rest) = message.strip_prefix("Fabric installed for Minecraft ") {
        return format!("Fabric 已安装到 Minecraft {rest}");
    }
    if let Some(title) = message.strip_suffix(" completed") {
        if let Some(slug) = title.strip_prefix("Install mod ") {
            return format!("模组 {slug} 安装完成");
        }
        return format!("{title} 已完成");
    }
    if message == "Task cancelled" {
        return "任务已取消".into();
    }
    message.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes_and_labels_round_trip() {
        assert_eq!(Lang::from_code("en"), Lang::English);
        assert_eq!(Lang::from_code("zh-CN"), Lang::Chinese);
        assert_eq!(Lang::from_code("中文"), Lang::Chinese);
        assert_eq!(Lang::from_code("nonsense"), Lang::English);
        assert_eq!(Lang::English.code(), "en");
        assert_eq!(Lang::Chinese.label(), "中文");
        assert_eq!(Lang::English.next(), Lang::Chinese);
        assert_eq!(Lang::Chinese.next(), Lang::English);
        assert_eq!(Lang::Chinese.pick("Play", "启动"), "启动");
    }

    #[test]
    fn job_phases_are_mapped_but_unknown_text_is_preserved() {
        assert_eq!(
            localize_job_phase(Lang::Chinese, "Downloading assets"),
            "下载资源文件"
        );
        assert_eq!(
            localize_job_phase(Lang::Chinese, "installer output"),
            "installer output"
        );
        assert_eq!(
            localize_job_result(Lang::Chinese, "Minecraft 1.21.4 is ready to play"),
            "Minecraft 1.21.4 已就绪，可以启动"
        );
        assert_eq!(
            localize_job_result(Lang::Chinese, "Install mod sodium completed"),
            "模组 sodium 安装完成"
        );
        assert_eq!(
            localize_job_result(Lang::Chinese, "Forge 21.1.0 completed"),
            "Forge 21.1.0 已完成"
        );
    }
}
