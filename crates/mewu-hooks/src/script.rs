// SPDX-License-Identifier: MPL-2.0
use crate::{Event, Result, MAX_SCRIPT_BYTES};
use rquickjs::{Context, Module, Runtime};
use serde_json::Value;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
/// Parse a function body without evaluating its condition or invoking effects.
pub fn validate_script(code: &str) -> Result<()> {
    if code.len() > MAX_SCRIPT_BYTES {
        return Err("脚本内容过大".into());
    }
    let rt = Runtime::new().map_err(|_| "无法校验脚本")?;
    rt.set_memory_limit(16 * 1024 * 1024);
    rt.set_max_stack_size(256 * 1024);
    let until = Instant::now() + Duration::from_millis(100);
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() >= until)));
    let ctx = Context::full(&rt).map_err(|_| "无法校验脚本")?;
    // Module::declare uses JS_EVAL_FLAG_COMPILE_ONLY. QuickJS's Function
    // constructor currently reparses a wrapper with eval (upstream XXX), so
    // it cannot serve as a parse-only validator for user-authored bodies.
    ctx.with(|ctx| Module::declare(ctx, "hook.mjs", source(code)).map(|_| ()))
        .map_err(|_| "JavaScript 语法无效".into())
}
fn source(code: &str) -> String {
    format!("export default function hook(event){{'use strict';\n{code}\n}}")
}
/// A fresh pure ECMAScript context. No module loader, process, filesystem,
/// network, timers or host bridge. Code comes only from a saved local rule;
/// event contents are passed as a JSON value, never interpolated as code.
pub fn evaluate(code: &str, event: &Event, cancel: Arc<AtomicBool>) -> Result<Value> {
    event.validate()?;
    if code.len() > MAX_SCRIPT_BYTES || cancel.load(Ordering::Acquire) {
        return Err("脚本已取消或内容过大".into());
    }
    let rt = Runtime::new().map_err(|_| "无法创建脚本运行时")?;
    rt.set_memory_limit(16 * 1024 * 1024);
    rt.set_max_stack_size(256 * 1024);
    rt.set_gc_threshold(4 * 1024 * 1024);
    let until = Instant::now() + Duration::from_millis(100);
    rt.set_interrupt_handler(Some(Box::new(move || {
        cancel.load(Ordering::Acquire) || Instant::now() >= until
    })));
    let ctx = Context::full(&rt).map_err(|_| "无法创建脚本环境")?;
    let out = ctx
        .with(|ctx| -> rquickjs::Result<String> {
            let input = serde_json::to_string(event).map_err(|_| rquickjs::Error::Unknown)?;
            let event = ctx.json_parse(input)?;
            let (module, ready) = Module::declare(ctx.clone(), "hook.mjs", source(code))?.eval()?;
            ready.finish::<()>()?;
            let function: rquickjs::Function = module.get("default")?;
            let output: rquickjs::Value = function.call((event,))?;
            ctx.json_stringify(output)?
                .ok_or(rquickjs::Error::Unknown)?
                .to_string()
        })
        .map_err(|_| "JavaScript 执行失败或超出限制")?;
    if out.len() > 64 * 1024 {
        return Err("JavaScript 返回内容过大".into());
    }
    serde_json::from_str(&out)
        .map_err(|_| "JavaScript 必须返回 JSON 值；条件须返回 true 或 false".into())
}
