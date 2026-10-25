// SPDX-License-Identifier: MPL-2.0
use chrono::{DateTime, Utc};
use croner::Cron;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::str::FromStr;

pub const MAX_RULES: usize = 256;
pub const MAX_EVENT_BYTES: usize = 256 * 1024;
pub const MAX_SCRIPT_BYTES: usize = 64 * 1024;
pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub enabled: bool,
    pub trigger: Trigger,
    #[serde(default)]
    pub condition: String,
    pub actions: Vec<Action>,
    #[serde(default = "cooldown")]
    pub cooldown_ms: u64,
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub stop_on_error: bool,
}
fn cooldown() -> u64 {
    500
}
fn timeout() -> u64 {
    10000
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Trigger {
    Cron {
        expression: String,
        timezone: String,
    },
    Prompt {
        matcher: Matcher,
    },
    Reply {
        matcher: Matcher,
    },
    Shortcut {
        accelerator: String,
    },
    Event {
        name: String,
    },
}
impl Trigger {
    pub fn category(&self) -> &'static str {
        match self {
            Self::Cron { .. } => "cron",
            Self::Prompt { .. } => "prompt",
            Self::Reply { .. } => "reply",
            Self::Shortcut { .. } => "shortcut",
            Self::Event { .. } => "event",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Matcher {
    pub mode: MatchMode,
    pub value: String,
    #[serde(default)]
    pub ignore_case: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MatchMode {
    Contains,
    Exact,
    Regex,
}
impl Matcher {
    fn regex(&self) -> Result<regex::Regex> {
        regex::RegexBuilder::new(&self.value)
            .case_insensitive(self.ignore_case)
            .size_limit(1024 * 1024)
            .dfa_size_limit(1024 * 1024)
            .build()
            .map_err(|_| "匹配表达式无效".into())
    }
    pub fn matches(&self, text: &str) -> Result<bool> {
        Ok(match self.mode {
            MatchMode::Regex => self.regex()?.is_match(text),
            MatchMode::Exact if self.ignore_case => {
                text.to_lowercase() == self.value.to_lowercase()
            }
            MatchMode::Contains if self.ignore_case => {
                text.to_lowercase().contains(&self.value.to_lowercase())
            }
            MatchMode::Exact => text == self.value,
            MatchMode::Contains => text.contains(&self.value),
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Action {
    InjectPrompt {
        text: String,
    },
    SendPrompt {
        text: String,
        #[serde(default)]
        scene_id: Option<String>,
    },
    Mcp {
        server_id: String,
        tool: String,
        arguments: Value,
    },
    Command {
        shell: Shell,
        code: String,
        #[serde(default)]
        cwd: Option<String>,
    },
    Program {
        executable: String,
        arguments: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
    },
    OpenUrl {
        url: String,
    },
    OpenApp {
        path: String,
    },
    Input {
        steps: Vec<InputStep>,
    },
    Script {
        code: String,
    },
    Emit {
        name: String,
        payload: Value,
    },
    App {
        method: String,
        params: Value,
    },
    Delay {
        milliseconds: u64,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Shell {
    Cmd,
    Powershell,
    Sh,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum InputStep {
    Move {
        x: i32,
        y: i32,
    },
    Click {
        button: MouseButton,
        #[serde(default = "clicks")]
        count: u8,
    },
    Button {
        button: MouseButton,
        direction: ButtonDirection,
    },
    Key {
        key: String,
    },
    Text {
        text: String,
    },
    Scroll {
        x: i32,
        y: i32,
    },
    Delay {
        milliseconds: u64,
    },
}
fn clicks() -> u8 {
    1
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ButtonDirection {
    Press,
    Release,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Event {
    pub id: String,
    pub name: String,
    pub timestamp: i64,
    pub platform: String,
    #[serde(default)]
    pub scene_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub payload: Value,
    // Only the native event bus assigns ancestry. External emitters must not
    // deserialize this type as authorization; depth alone is not a grant.
    #[serde(default)]
    pub ancestors: Vec<String>,
    #[serde(default)]
    pub depth: u8,
}
impl Event {
    pub fn new(name: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            timestamp: Utc::now().timestamp_millis(),
            platform: std::env::consts::OS.into(),
            scene_id: None,
            run_id: None,
            text: String::new(),
            payload: Value::Null,
            ancestors: vec![],
            depth: 0,
        }
    }
    pub fn validate(&self) -> Result<()> {
        valid_uuid(&self.id)?;
        valid_event_name(&self.name)?;
        if !matches!(self.platform.as_str(), "windows" | "macos" | "linux") {
            return Err("事件平台无效".into());
        }
        if self.ancestors.len() > MAX_RULES
            || self.depth > 8
            || serde_json::to_vec(self).map_err(|_| "事件格式无效")?.len() > MAX_EVENT_BYTES
        {
            return Err("事件内容过大或触发层级过深".into());
        }
        Ok(())
    }
}
pub fn valid_uuid(value: &str) -> Result<()> {
    if uuid::Uuid::parse_str(value).is_ok_and(|v| !v.is_nil() && v.to_string() == value) {
        Ok(())
    } else {
        Err("规则标识无效".into())
    }
}
pub fn valid_event_name(name: &str) -> Result<()> {
    if !name.is_empty()
        && name.len() <= 120
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        Ok(())
    } else {
        Err("事件名称无效".into())
    }
}
fn bounded(s: &str, max: usize, label: &str) -> Result<()> {
    if s.trim().is_empty() || s.len() > max || s.contains('\0') {
        Err(format!("{label}无效"))
    } else {
        Ok(())
    }
}
pub fn validate_action(a: &Action) -> Result<()> {
    match a {
        Action::InjectPrompt { text } | Action::SendPrompt { text, .. } => {
            bounded(text, 64 * 1024, "提示词")?
        }
        Action::Mcp {
            server_id,
            tool,
            arguments,
        } => {
            bounded(server_id, 160, "MCP 服务")?;
            bounded(tool, 160, "MCP 工具")?;
            if !arguments.is_object() {
                return Err("MCP 参数必须为 JSON 对象".into());
            }
        }
        Action::Command { code, cwd, .. } => {
            bounded(code, MAX_SCRIPT_BYTES, "命令")?;
            if let Some(cwd) = cwd {
                bounded(cwd, 4096, "工作目录")?
            }
        }
        Action::Program {
            executable,
            arguments,
            cwd,
        } => {
            bounded(executable, 4096, "程序")?;
            if arguments.len() > 128 || arguments.iter().any(|x| x.len() > 8192 || x.contains('\0'))
            {
                return Err("程序参数无效".into());
            }
            if let Some(cwd) = cwd {
                bounded(cwd, 4096, "工作目录")?
            }
        }
        Action::OpenUrl { url } => {
            bounded(url, 8192, "网址")?;
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err("网址必须使用 HTTP 或 HTTPS".into());
            }
        }
        Action::OpenApp { path } => bounded(path, 4096, "应用路径")?,
        Action::Input { steps } => {
            if steps.is_empty() || steps.len() > 128 {
                return Err("输入操作须为 1 至 128 步".into());
            }
            for s in steps {
                match s {
                    InputStep::Key { key } => bounded(key, 80, "按键")?,
                    InputStep::Text { text } => bounded(text, 8192, "输入文字")?,
                    InputStep::Click { count, .. } if !(1..=3).contains(count) => {
                        return Err("点击次数无效".into())
                    }
                    InputStep::Scroll { x, y }
                        if !(-1000..=1000).contains(x) || !(-1000..=1000).contains(y) =>
                    {
                        return Err("单步滚动范围过大".into())
                    }
                    InputStep::Delay { milliseconds } if *milliseconds > 10000 => {
                        return Err("等待时间过长".into())
                    }
                    _ => {}
                }
            }
        }
        Action::Script { code } => bounded(code, MAX_SCRIPT_BYTES, "JavaScript")?,
        Action::Emit { name, .. } => {
            valid_event_name(name)?;
            if !name.starts_with("custom.") {
                return Err("自定义事件须使用 custom. 前缀".into());
            }
        }
        Action::App { method, params } => {
            bounded(method, 120, "Mewu 操作")?;
            if !params.is_object() {
                return Err("操作参数必须为 JSON 对象".into());
            }
        }
        Action::Delay { milliseconds } => {
            if *milliseconds > 10000 {
                return Err("等待时间过长".into());
            }
        }
    }
    if serde_json::to_vec(a).map_err(|_| "动作格式无效")?.len() > 128 * 1024 {
        return Err("动作内容过大".into());
    }
    Ok(())
}
impl Rule {
    pub fn validate(&self) -> Result<()> {
        valid_uuid(&self.id)?;
        bounded(&self.name, 240, "名称")?;
        if self.revision > i64::MAX as u64
            || self.condition.len() > MAX_SCRIPT_BYTES
            || !(100..=60000).contains(&self.timeout_ms)
            || self.cooldown_ms > 86400000
        {
            return Err("规则限制无效".into());
        }
        match &self.trigger {
            Trigger::Cron {
                expression,
                timezone,
            } => {
                bounded(expression, 256, "Cron")?;
                next_occurrence(expression, timezone, Utc::now())?;
            }
            Trigger::Prompt { matcher } | Trigger::Reply { matcher } => {
                bounded(&matcher.value, 4096, "匹配内容")?;
                if matcher.mode == MatchMode::Regex {
                    matcher.regex()?;
                }
            }
            Trigger::Shortcut { accelerator } => bounded(accelerator, 120, "快捷键")?,
            Trigger::Event { name } => valid_event_name(name)?,
        }
        if self.actions.is_empty() || self.actions.len() > 16 {
            return Err("每条规则须有 1 至 16 个动作".into());
        }
        for a in &self.actions {
            validate_action(a)?;
        }
        if self
            .actions
            .iter()
            .any(|a| matches!(a, Action::InjectPrompt { .. }))
            && !matches!(self.trigger, Trigger::Prompt { .. })
        {
            return Err("注入提示词仅适用于提示词触发；其他触发可使用发送提示词".into());
        }
        if serde_json::to_vec(self).map_err(|_| "规则格式无效")?.len() > 256 * 1024 {
            return Err("规则内容过大".into());
        }
        Ok(())
    }
    pub fn matches(&self, event: &Event) -> Result<bool> {
        if !self.enabled || event.ancestors.iter().any(|v| v == &self.id) || event.depth >= 8 {
            return Ok(false);
        }
        match &self.trigger {
            Trigger::Prompt { matcher } => {
                Ok(event.name == "prompt.beforeSend" && matcher.matches(&event.text)?)
            }
            Trigger::Reply { matcher } => {
                Ok(event.name == "agent.replyCompleted" && matcher.matches(&event.text)?)
            }
            Trigger::Event { name } => Ok(event.name == *name),
            Trigger::Cron { .. } => Ok(event.name == "schedule.due"
                && event.payload.get("ruleId").and_then(Value::as_str) == Some(&self.id)
                && event.payload.get("ruleRevision").and_then(Value::as_u64)
                    == Some(self.revision)),
            Trigger::Shortcut { .. } => Ok(event.name == "shortcut.pressed"
                && event.payload.get("ruleId").and_then(Value::as_str) == Some(&self.id)
                && event.payload.get("ruleRevision").and_then(Value::as_u64)
                    == Some(self.revision)),
        }
    }
}
pub fn next_occurrence(
    expression: &str,
    timezone: &str,
    after: DateTime<Utc>,
) -> Result<DateTime<Utc>> {
    let cron = Cron::from_str(expression).map_err(|_| "Cron 表达式无效")?;
    if timezone == "local" {
        cron.find_next_occurrence(&after.with_timezone(&chrono::Local), false)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|_| "Cron 没有后续执行时间".into())
    } else {
        let tz = chrono_tz::Tz::from_str(timezone).map_err(|_| "时区无效，请使用 IANA 时区")?;
        cron.find_next_occurrence(&after.with_timezone(&tz), false)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|_| "Cron 没有后续执行时间".into())
    }
}
