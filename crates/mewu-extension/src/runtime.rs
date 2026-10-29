// SPDX-License-Identifier: MPL-2.0
use crate::{
    contract::*,
    package::{module_path, validate_package, ExtensionPackage},
};
use rquickjs::{
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
    Context, Ctx, Function, Module, Runtime,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub const SDK_SOURCE: &str = include_str!("sdk.js");
const BOOTSTRAP_MODULE: &str = "@mewu/bootstrap";

/// Compile and resolve the entry's static dependencies with the runtime parser.
/// COMPILE_ONLY + ResolveModule never evaluates plugin/module bodies, invokes
/// activation, pumps jobs, or installs a host bridge. Incomplete archives must
/// fail before the installed plugin or its state can be replaced.
pub fn validate_module_graph(package:&ExtensionPackage)->Result<(),ExtensionError>{
    validate_package(&package.manifest,&package.modules)?;
    let invalid=||ExtensionError::InvalidPackage("JavaScript module graph is invalid or incomplete");
    let runtime=Runtime::new().map_err(|_|invalid())?;
    runtime.set_memory_limit(32*1024*1024);
    runtime.set_max_stack_size(512*1024);
    let expires=Instant::now()+Duration::from_secs(3);
    runtime.set_interrupt_handler(Some(Box::new(move||Instant::now()>=expires)));
    runtime.set_loader(PackageResolver{names:package.modules.keys().cloned().collect()},PackageLoader{modules:package.modules.clone()});
    let context=Context::full(&runtime).map_err(|_|invalid())?;
    context.with(|ctx|{
        let source=std::ffi::CString::new(package.modules[&package.manifest.entry].as_str()).map_err(|_|invalid())?;
        let name=std::ffi::CString::new(package.manifest.entry.as_str()).map_err(|_|invalid())?;
        // Both CStrings and their validated lengths outlive these calls. The
        // returned JSValue is released once, including failed dependency resolution.
        let value=unsafe{rquickjs::qjs::JS_Eval(ctx.as_raw().as_ptr(),source.as_ptr(),source.as_bytes().len() as _,name.as_ptr(),
            (rquickjs::qjs::JS_EVAL_TYPE_MODULE|rquickjs::qjs::JS_EVAL_FLAG_STRICT|rquickjs::qjs::JS_EVAL_FLAG_COMPILE_ONLY) as i32)};
        if unsafe{rquickjs::qjs::JS_IsException(value)} {drop(ctx.catch());return Err(invalid())}
        let linked=unsafe{rquickjs::qjs::JS_ResolveModule(ctx.as_raw().as_ptr(),value)};
        unsafe{rquickjs::qjs::JS_FreeValue(ctx.as_raw().as_ptr(),value)};
        if linked<0{drop(ctx.catch());return Err(invalid())}
        Ok(())
    })
}

#[derive(Debug, Clone)]
pub struct RuntimeLimits {
    pub memory_bytes: usize,
    pub stack_bytes: usize,
    /// Module loading/activation have a distinct budget. A host can assign a
    /// short service invocation deadline without timing out interpreter setup.
    pub initialization_timeout: Duration,
    pub request_timeout: Duration,
    pub max_host_calls: usize,
}
impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 32 * 1024 * 1024,
            stack_bytes: 512 * 1024,
            initialization_timeout: Duration::from_secs(20),
            request_timeout: Duration::from_secs(90),
            max_host_calls: 512,
        }
    }
}

struct PackageResolver {
    names: BTreeSet<String>,
}
impl Resolver for PackageResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        if attributes.is_some() {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                "Import attributes are unsupported",
            ));
        }
        if name == "@mewu/sdk" {
            return Ok(name.into());
        }
        if !(name.starts_with("./") || name.starts_with("../"))
            || name.contains('\\')
            || name.contains(':')
            || name.contains('\0')
        {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                "Only package relative modules and @mewu/sdk are allowed",
            ));
        }
        let directory = if base == BOOTSTRAP_MODULE {
            ""
        } else {
            base.rsplit_once('/').map(|v| v.0).unwrap_or("")
        };
        let mut segments: Vec<&str> = directory.split('/').filter(|v| !v.is_empty()).collect();
        for segment in name.split('/') {
            match segment {
                "." => {}
                ".." => {
                    if segments.pop().is_none() {
                        return Err(rquickjs::Error::new_resolving_message(
                            base,
                            name,
                            "Module path escaped package",
                        ));
                    }
                }
                "" => {
                    return Err(rquickjs::Error::new_resolving_message(
                        base,
                        name,
                        "Empty module path",
                    ))
                }
                other => segments.push(other),
            }
        }
        let resolved = segments.join("/");
        if !module_path(&resolved) || !self.names.contains(&resolved) {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                "Package module does not exist",
            ));
        }
        Ok(resolved)
    }
}
struct PackageLoader {
    modules: BTreeMap<String, String>,
}
impl Loader for PackageLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        if attributes.is_some() {
            return Err(rquickjs::Error::new_loading_message(
                name,
                "Import attributes are unsupported",
            ));
        }
        let source = if name == "@mewu/sdk" {
            SDK_SOURCE
        } else {
            self.modules.get(name).map(String::as_str).ok_or_else(|| {
                rquickjs::Error::new_loading_message(name, "Package module does not exist")
            })?
        };
        Module::declare(ctx.clone(), name, source)
    }
}

type Output = Arc<dyn Fn(WorkerResponse) -> Result<(), ExtensionError> + Send + Sync>;

/// Not Send: one worker thread owns the QuickJS context for its entire lifetime.
pub struct ExtensionRuntime {
    context: Context,
    runtime: Runtime,
    deadline: Arc<Mutex<Instant>>,
    canceled: Arc<AtomicBool>,
    pending: Arc<Mutex<BTreeSet<String>>>,
    host_count: Arc<AtomicU64>,
    limits: RuntimeLimits,
    registrations: Vec<ServiceRegistration>,
    permissions: Vec<String>,
}

impl ExtensionRuntime {
    pub fn new(
        package: ExtensionPackage,
        limits: RuntimeLimits,
        canceled: Arc<AtomicBool>,
        output: Output,
    ) -> Result<Self, ExtensionError> {
        validate_package(&package.manifest, &package.modules)?;
        if package.manifest.requires_capabilities.iter().any(|value|!HOST_CAPABILITIES.contains(&value.as_str())) {
            return Err(ExtensionError::InvalidPackage("required host capability unavailable"));
        }
        if !(4 * 1024 * 1024..=256 * 1024 * 1024).contains(&limits.memory_bytes)
            || !(64 * 1024..=2 * 1024 * 1024).contains(&limits.stack_bytes)
            || limits.initialization_timeout.is_zero()
            || limits.initialization_timeout > Duration::from_secs(300)
            || limits.request_timeout.is_zero()
            || limits.request_timeout > Duration::from_secs(300)
            || limits.max_host_calls == 0
            || limits.max_host_calls > 4096
        {
            return Err(ExtensionError::Runtime("invalid interpreter limits"));
        }
        let runtime =
            Runtime::new().map_err(|_| ExtensionError::Runtime("cannot create interpreter"))?;
        runtime.set_memory_limit(limits.memory_bytes);
        runtime.set_max_stack_size(limits.stack_bytes);
        runtime.set_gc_threshold(limits.memory_bytes / 2);
        let deadline = Arc::new(Mutex::new(Instant::now() + limits.initialization_timeout));
        let deadline_signal = deadline.clone();
        let cancel_signal = canceled.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            cancel_signal.load(Ordering::Acquire)
                || deadline_signal
                    .lock()
                    .map_or(true, |deadline| Instant::now() >= *deadline)
        })));
        runtime.set_loader(
            PackageResolver {
                names: package.modules.keys().cloned().collect(),
            },
            PackageLoader {
                modules: package.modules,
            },
        );
        let context = Context::full(&runtime)
            .map_err(|_| ExtensionError::Runtime("cannot create interpreter context"))?;
        let pending = Arc::new(Mutex::new(BTreeSet::new()));
        let pending_calls = pending.clone();
        let host_count = Arc::new(AtomicU64::new(0));
        let call_count = host_count.clone();
        let call_serial = Arc::new(AtomicU64::new(1));
        let max_host_calls = limits.max_host_calls;
        context
            .with(|ctx| -> rquickjs::Result<()> {
                let send = Function::new(
                    ctx.clone(),
                    move |method: String, params: String| -> rquickjs::Result<String> {
                        if !identifier(&method)
                            || params.len() > MAX_VALUE_BYTES
                            || call_count.fetch_add(1, Ordering::AcqRel) >= max_host_calls as u64
                        {
                            return Err(rquickjs::Error::Unknown);
                        }
                        let params: Value =
                            serde_json::from_str(&params).map_err(|_| rquickjs::Error::Unknown)?;
                        validate_value(&params).map_err(|_| rquickjs::Error::Unknown)?;
                        let id = format!("host-{}", call_serial.fetch_add(1, Ordering::Relaxed));
                        let mut pending =
                            pending_calls.lock().map_err(|_| rquickjs::Error::Unknown)?;
                        if pending.len() >= 64 {
                            return Err(rquickjs::Error::Unknown);
                        }
                        pending.insert(id.clone());
                        if output(WorkerResponse::HostCall {
                            id: id.clone(),
                            method,
                            params,
                        })
                        .is_err()
                        {
                            pending.remove(&id);
                            return Err(rquickjs::Error::Unknown);
                        }
                        Ok(id)
                    },
                )?;
                ctx.globals().set("__mewuSend", send)?;
                // No OS, Node, fetch, filesystem, process or native-module globals are
                // installed. QuickJS's pure ECMAScript intrinsics are available.
                let entry = serde_json::to_string(&format!("./{}", package.manifest.entry))
                    .map_err(|_| rquickjs::Error::Unknown)?;
                let bootstrap = format!(
                    r#"
                import {{ createContext }} from '@mewu/sdk';
                import * as plugin from {entry};
                const scope = createContext(globalThis.__mewuSend);
                delete globalThis.__mewuSend;
                globalThis.__mewuScope = scope;
                globalThis.__mewuActivation = 'pending';
                const activate = plugin.default ?? plugin.activate;
                if (typeof activate !== 'function') throw new Error('Missing activation entry');
                Promise.resolve().then(() => activate(scope.ctx)).then(() => {{
                  globalThis.__mewuRegistrations = scope.finishRegistration();
                  globalThis.__mewuActivation = 'ready';
                }}).catch(() => {{ globalThis.__mewuActivation = 'failed'; }});
            "#
                );
                let _promise = Module::evaluate(ctx.clone(), BOOTSTRAP_MODULE, bootstrap)?;
                Ok(())
            })
            .map_err(|_| {
                if canceled.load(Ordering::Acquire) {
                    ExtensionError::Canceled
                } else if Instant::now() >= *deadline.lock().unwrap() {
                    ExtensionError::Timeout
                } else {
                    ExtensionError::Runtime("module activation failed")
                }
            })?;
        Ok(Self {
            context,
            runtime,
            deadline,
            canceled,
            pending,
            host_count,
            limits,
            registrations: Vec::new(),
            permissions: package.manifest.permissions.clone(),
        })
    }

    pub fn check_budget(&self) -> Result<(), ExtensionError> {
        if self.canceled.load(Ordering::Acquire) {
            return Err(ExtensionError::Canceled);
        }
        if Instant::now()
            >= *self
                .deadline
                .lock()
                .map_err(|_| ExtensionError::Runtime("interpreter state unavailable"))?
        {
            return Err(ExtensionError::Timeout);
        }
        Ok(())
    }

    pub fn pump(&self) -> Result<(), ExtensionError> {
        self.check_budget()?;
        // A hostile Promise chain must not monopolize transport/cancellation.
        for _ in 0..64 {
            if !self
                .runtime
                .execute_pending_job()
                .map_err(|_| self.js_error("JavaScript job failed"))?
            {
                break;
            }
            self.check_budget()?;
        }
        Ok(())
    }

    fn js_error(&self, message: &'static str) -> ExtensionError {
        self.check_budget()
            .err()
            .unwrap_or(ExtensionError::Runtime(message))
    }

    pub fn poll_initialization(
        &mut self,
    ) -> Result<Option<Vec<ServiceRegistration>>, ExtensionError> {
        self.pump()?;
        let state: String = self
            .context
            .with(|ctx| ctx.globals().get("__mewuActivation"))
            .map_err(|_| self.js_error("activation state unavailable"))?;
        if state == "pending" {
            return Ok(None);
        }
        if state != "ready" {
            return Err(ExtensionError::Runtime("plugin activation failed"));
        }
        let registrations: String = self
            .context
            .with(|ctx| ctx.globals().get("__mewuRegistrations"))
            .map_err(|_| self.js_error("service registrations unavailable"))?;
        if registrations.len() > MAX_VALUE_BYTES {
            return Err(ExtensionError::Runtime(
                "service registrations exceed limit",
            ));
        }
        let registrations: Vec<ServiceRegistration> = serde_json::from_str(&registrations)
            .map_err(|_| ExtensionError::Runtime("invalid service registrations"))?;
        if registrations.len() > MAX_SERVICES {
            return Err(ExtensionError::Runtime("service count exceeds limit"));
        }
        let mut ids = BTreeSet::new();
        for registration in &registrations {
            crate::service_platforms(registration)?;
            crate::validate_trigger(registration)?;
            if crate::appearance_theme(registration)?.is_some() && !self.permissions.iter().any(|v|v==crate::APPEARANCE_PERMISSION) {
                return Err(ExtensionError::InvalidPackage("appearance contribution requires ui.theme"));
            }
            if !identifier(&registration.id)
                || !ids.insert(&registration.id)
                || !bounded_text(&registration.title, 128, false)
                || registration
                    .description
                    .as_ref()
                    .is_some_and(|s| !bounded_text(s, 4000, true))
            {
                return Err(ExtensionError::Runtime(
                    "invalid or duplicate service registration",
                ));
            }
            for schema in [&registration.input_schema, &registration.metadata]
                .into_iter()
                .flatten()
            {
                validate_value(schema)?;
                if !schema.is_object() {
                    return Err(ExtensionError::Runtime(
                        "service schema and metadata must be objects",
                    ));
                }
            }
        }
        self.registrations = registrations.clone();
        Ok(Some(registrations))
    }

    pub fn begin_invoke(
        &self,
        service_id: &str,
        input: &Value,
        context: &Value,
    ) -> Result<(), ExtensionError> {
        if !self.registrations.iter().any(|s| s.id == service_id) {
            return Err(ExtensionError::Protocol("unknown service"));
        }
        if !self
            .pending
            .lock()
            .map_err(|_| ExtensionError::Runtime("interpreter state unavailable"))?
            .is_empty()
        {
            return Err(ExtensionError::Runtime(
                "unsettled host calls from previous request",
            ));
        }
        validate_value(input)?;
        validate_value(context)?;
        *self
            .deadline
            .lock()
            .map_err(|_| ExtensionError::Runtime("interpreter state unavailable"))? =
            Instant::now() + self.limits.request_timeout;
        self.host_count.store(0, Ordering::Release);
        self.context
            .with(|ctx| {
                let scope: rquickjs::Object = ctx.globals().get("__mewuScope")?;
                let invoke: Function = scope.get("invoke")?;
                invoke.call::<_, ()>((
                    service_id,
                    serde_json::to_string(input).unwrap(),
                    serde_json::to_string(context).unwrap(),
                ))
            })
            .map_err(|_| self.js_error("service invocation failed"))
    }

    pub fn poll_result(&self) -> Result<Option<Value>, ExtensionError> {
        self.pump()?;
        let outcome: Option<String> = self
            .context
            .with(|ctx| {
                let scope: rquickjs::Object = ctx.globals().get("__mewuScope")?;
                let take: Function = scope.get("takeOutcome")?;
                take.call(())
            })
            .map_err(|_| self.js_error("service result unavailable"))?;
        let Some(outcome) = outcome else {
            return Ok(None);
        };
        if outcome.len() > MAX_VALUE_BYTES {
            return Err(ExtensionError::Runtime("service result exceeds limit"));
        }
        let outcome: Value = serde_json::from_str(&outcome)
            .map_err(|_| ExtensionError::Runtime("service returned invalid JSON"))?;
        if outcome.get("ok") != Some(&Value::Bool(true)) {
            return Err(ExtensionError::Handler(HandlerFailureKind::from_outcome(
                outcome.get("failureKind").and_then(Value::as_str),
            )));
        }
        let result = outcome.get("result").cloned().unwrap_or(Value::Null);
        validate_value(&result)?;
        if !self
            .pending
            .lock()
            .map_err(|_| ExtensionError::Runtime("interpreter state unavailable"))?
            .is_empty()
        {
            return Err(ExtensionError::Runtime("handler left unsettled host calls"));
        }
        Ok(Some(result))
    }

    pub fn host_result(
        &self,
        id: &str,
        result: Option<Value>,
        error: Option<ProtocolError>,
    ) -> Result<(), ExtensionError> {
        if !identifier(id) || (result.is_some() && error.is_some()) {
            return Err(ExtensionError::Protocol("invalid host response"));
        }
        if !self
            .pending
            .lock()
            .map_err(|_| ExtensionError::Runtime("interpreter state unavailable"))?
            .remove(id)
        {
            return Err(ExtensionError::Protocol("unknown host response"));
        }
        // Host errors already carry a bounded public code. Preserve that code
        // for plugin recovery, never forward transport bodies or error text.
        let result = if let Some(error) = &error {
            let code = if !error.code.is_empty() && error.code.len() <= 64
                && error.code.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
                error.code.as_str()
            } else { "HOST_ERROR" };
            serde_json::json!({"code":code})
        } else { result.unwrap_or(Value::Null) };
        validate_value(&result)?;
        self.context
            .with(|ctx| {
                let scope: rquickjs::Object = ctx.globals().get("__mewuScope")?;
                let settle: Function = scope.get("settle")?;
                settle.call::<_, ()>((id, serde_json::to_string(&result).unwrap(), error.is_some()))
            })
            .map_err(|_| self.js_error("host response delivery failed"))
    }

    pub fn deactivate(&self) -> Result<(), ExtensionError> {
        *self
            .deadline
            .lock()
            .map_err(|_| ExtensionError::Runtime("interpreter state unavailable"))? =
            Instant::now() + Duration::from_millis(500);
        self.context
            .with(|ctx| -> rquickjs::Result<()> {
                ctx.eval::<(), _>("void __mewuScope.deactivate()")?;
                Ok(())
            })
            .map_err(|_| self.js_error("plugin deactivation failed"))?;
        loop {
            self.pump()?;
            let done = self
                .context
                .with(|ctx| {
                    let scope: rquickjs::Object = ctx.globals().get("__mewuScope")?;
                    let finished: Function = scope.get("deactivationDone")?;
                    finished.call::<_, bool>(())
                })
                .map_err(|_| self.js_error("deactivation state unavailable"))?;
            if done {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
