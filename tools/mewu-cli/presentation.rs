// SPDX-License-Identifier: MPL-2.0
//! Console presentation only; never changes transport, dispatch or receipts.
use super::{Action, Invocation, Response};
use serde_json::Value;
use std::{
    ffi::OsString,
    io::{self, Write},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OutputFormat {
    Text,
    Json,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum HelpTopic {
    Root,
    Status,
    Version,
    Schema,
    Capabilities,
    Scene,
    SceneList,
    SceneGet,
    Run,
    RunGet,
    Call,
}
impl HelpTopic {
    pub(super) fn from_words(words: &[&str]) -> Option<Self> {
        Some(match words {
            [] => Self::Root,
            ["status"] => Self::Status,
            ["version"] => Self::Version,
            ["schema"] => Self::Schema,
            ["capabilities"] => Self::Capabilities,
            ["scene"] => Self::Scene,
            ["scene", "list"] => Self::SceneList,
            ["scene", "get"] | ["scene", "get", _] => Self::SceneGet,
            ["run"] => Self::Run,
            ["run", "get"] | ["run", "get", _] => Self::RunGet,
            ["call"] | ["call", _] => Self::Call,
            _ => return None,
        })
    }
}

pub(super) fn help(topic: HelpTopic) -> String {
    let body = match topic {
        HelpTopic::Root => "用法\n  mewu <命令> [选项]\n\n常用命令\n  status          查看运行状态\n  scene list      列出会话\n  scene get <ID>  查看会话内容\n  run get <ID>    查看 AI 回复与任务状态\n  call <方法>     调用截图、绘制、录屏等能力\n  capabilities    列出可调用的方法\n  schema          输出完整接口 JSON\n  version         查看版本\n\n示例\n  mewu status\n  mewu call capture.open --wait\n  mewu scene list --output json\n\n使用 mewu <命令> --help 查看详细用法。",
        HelpTopic::Status => "用法\n  mewu status [--output text|json]\n\n查看正在运行的 Mewu 版本、平台、当前会话与 CLI 状态。\n需要先启动 Mewu，并开启“命令行 / CLI”插件。",
        HelpTopic::Version => "用法\n  mewu version [--output text|json]\n\n查看命令行客户端版本；不需要启动 Mewu。",
        HelpTopic::Schema => "用法\n  mewu schema\n\n从正在运行的 Mewu 获取完整接口 JSON，包含参数、结果和示例。\n仅查看方法列表可用 mewu capabilities。",
        HelpTopic::Capabilities => "用法\n  mewu capabilities [--output text|json]\n\n列出宿主公开的实际方法；参数和示例见 mewu schema。",
        HelpTopic::Scene => "用法\n  mewu scene list [--offset N] [--limit N]\n  mewu scene get <ID> [--limit N]\n\n列出会话，或读取指定会话最近的消息与当前状态。\n示例\n  mewu scene list --limit 10\n  mewu scene list --output json",
        HelpTopic::SceneList => "用法\n  mewu scene list [--offset N] [--limit N] [--output text|json]\n\n列出会话 ID、状态与标题。--offset 默认为 0，--limit 默认为 20。",
        HelpTopic::SceneGet => "用法\n  mewu scene get <ID> [--limit N] [--output text|json]\n\n查看会话与最近消息；--limit 默认取最近 20 条。\n从 mewu scene list 获取完整会话 ID。",
        HelpTopic::Run => "用法\n  mewu run get <会话ID> [--limit N] [--output text|json]\n\n查看指定会话的 AI 任务状态与最近消息。",
        HelpTopic::RunGet => "用法\n  mewu run get <会话ID> [--limit N] [--output text|json]\n\n查看 AI 任务状态与最近消息；--limit 默认取最近 20 条。\n会话 ID 从 mewu scene list 获取。",
        HelpTopic::Call => "用法\n  mewu call <方法> [参数选项] [--wait]\n\n参数输入（三选一）\n  --json JSON         直接传入参数对象\n  --file PATH         从 UTF-8 JSON 文件读取\n  --stdin             从标准输入读取\n\n执行选项\n  --wait              等待操作完成；只查询，不重新执行\n  --scene ID          指定会话，不覆盖参数中的目标\n  --revision N        指定场景修订号\n  --request-id ID     为本次操作指定唯一 ID\n  --timeout-ms MS     单次通信超时，默认 10000\n  --wait-timeout-ms MS 等待完成超时，默认 120000\n\n输出选项\n  --output text|json  选择文字或 JSON；call 默认输出 JSON\n  --jsonl             逐行输出实际响应进度，可配合 --wait\n\n示例\n  mewu call capture.open --wait\n  mewu call recording.status\n  mewu call chat.send --file message.json --wait\n\n使用 mewu capabilities 查看方法，mewu schema 查看参数。",
    };
    format!("MewuAI {}\n\n{body}\n", env!("CARGO_PKG_VERSION"))
}

pub(super) fn output_format(call: &Invocation, terminal: bool) -> OutputFormat {
    if call.jsonl {
        return OutputFormat::Json;
    }
    if let Some(output) = call.output {
        return output;
    }
    if matches!(call.action, Action::Help | Action::Version) {
        return OutputFormat::Text;
    }
    // Keep existing automation and the low-level API command machine-readable.
    if !terminal || call.raw_call || call.request.method == "app.schema" {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    }
}

pub(super) fn error_output(args: &[OsString], terminal: bool) -> OutputFormat {
    if args.iter().any(|a| a == "--jsonl") {
        return OutputFormat::Json;
    }
    for pair in args.windows(2) {
        if pair[0] == "--output" {
            if pair[1] == "json" {
                return OutputFormat::Json;
            }
            if pair[1] == "text" {
                return OutputFormat::Text;
            }
        }
    }
    if terminal {
        OutputFormat::Text
    } else {
        OutputFormat::Json
    }
}

// User titles/messages must not execute terminal escape sequences or overwrite
// prior output with CR, backspace, bidi overrides, or invisible control marks.
pub(super) fn safe_text(value: &str) -> String {
    value.chars().filter(|c| (!c.is_control() || *c == '\n' || *c == '\t')
        && !matches!(*c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')).collect()
}
fn field(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(s)) => safe_text(s),
        Some(Value::Null) | None => "—".into(),
        Some(v) => v.to_string(),
    }
}
fn single_line(value: &str) -> String {
    safe_text(value).replace(['\n', '\t'], " ")
}
fn scene_state(value: &Value) -> &'static str {
    if value["closed"] == true {
        "已关闭"
    } else if value["frozen"] == true {
        "已冻结"
    } else {
        "可用"
    }
}

pub(super) fn emit_text(
    writer: &mut impl Write,
    method: &str,
    response: &Response,
) -> io::Result<()> {
    // Human failures belong on stderr; do not dump the protocol envelope.
    if super::exit_code(response) != 0 {
        return Ok(());
    }
    let Some(value) = response.result() else {
        return Ok(());
    };
    if let Some(usage) = value.get("usage").and_then(Value::as_str) {
        writer.write_all(usage.as_bytes())?;
    } else if method == "version" {
        writeln!(writer, "MewuAI {}", field(value, "version"))?;
    } else {
        match method {
            "app.status" => {
                writeln!(writer, "MewuAI {} · {}\n\n状态      运行中\nCLI       已连接\n当前会话  {}\n修订号    {}",
                    field(value,"version"), field(value,"platform"), field(value,"activeSceneId"), field(value,"revision"))?;
            }
            "scene.list" => {
                if let Some(rows) = value["scenes"].as_array() {
                    if rows.is_empty() {
                        writeln!(writer, "暂无会话。")?;
                    } else {
                        writeln!(
                            writer,
                            "会话 ID                               状态      标题"
                        )?;
                        for row in rows {
                            writeln!(
                                writer,
                                "{}  {}  {}",
                                single_line(&field(row, "id")),
                                scene_state(row),
                                single_line(&field(row, "title"))
                            )?;
                        }
                    }
                    if let Some(offset) = value["nextOffset"].as_u64() {
                        writeln!(writer, "\n下一页: mewu scene list --offset {offset}")?;
                    }
                }
            }
            "scene.get" | "run.get" => {
                if method == "scene.get" {
                    writeln!(
                        writer,
                        "{}\n状态: {}",
                        single_line(&field(value, "title")),
                        scene_state(value)
                    )?;
                }
                writeln!(writer, "会话: {}", field(value, "sceneId"))?;
                let status = match value["run"]["status"].as_str() {
                    Some("running") => "回复中",
                    Some("completed") => "已完成",
                    Some("failed") => "失败",
                    Some("canceled") => "已取消",
                    _ => "无进行中的任务",
                };
                writeln!(writer, "AI: {status}")?;
                if let Some(messages) = value["messages"].as_array() {
                    for message in messages {
                        let role = if message["role"] == "assistant" {
                            "AI"
                        } else {
                            "你"
                        };
                        writeln!(writer, "\n{role}\n{}", field(message, "text"))?;
                    }
                }
            }
            "app.capabilities" => {
                if let Some(methods) = value["commands"].as_array() {
                    writeln!(writer, "可调用方法（{}）\n", methods.len())?;
                    for item in methods {
                        writeln!(
                            writer,
                            "  {:24} {}",
                            single_line(&field(item, "method")),
                            single_line(&field(item, "description"))
                        )?;
                    }
                    writeln!(writer, "\n参数与示例: mewu schema")?;
                }
            }
            _ => {
                let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
                writeln!(writer, "{text}")?;
            }
        }
    }
    writer.flush()
}

pub(super) fn error_message(code: i32) -> &'static str {
    match code {
        2 => "参数无效。运行 mewu --help 查看用法。",
        3 => "无法连接 Mewu。请先启动软件，并在插件中启用“命令行 / CLI”。",
        4 => "本机控制连接的身份或权限校验未通过。",
        5 => "请求超时。",
        6 => "操作结果未知，请查询原操作，不要直接重复执行。",
        74 => "无法写入命令行输出。",
        _ => "操作失败。可用 --output json 查看完整回执。",
    }
}
