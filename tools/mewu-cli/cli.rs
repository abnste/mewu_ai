// SPDX-License-Identifier: MPL-2.0
//! Console-only control client. No GUI startup, storage access, or mutation replay.
use mewu_control::{
    Client, ControlError, Endpoint, ErrorCode, ExecutionOutcome, Request, Response,
    MAX_FRAME_BYTES, PROTOCOL_VERSION,
};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::{json, Map, Number, Value};
use std::{
    collections::HashSet,
    ffi::OsString,
    fmt,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_TIMEOUT_MS: u64 = 3_600_000;
const POLL_INTERVAL: Duration = Duration::from_millis(200);
const SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const USAGE: &str = "mewu help | version | status | schema | capabilities\nmewu scene list [--offset N] [--limit N]\nmewu scene get SCENE_ID [--limit N]\nmewu run get SCENE_ID [--limit N]\nmewu call METHOD [--json JSON | --file PATH | --stdin]\nOptions: --request-id ID --scene ID --revision N --wait --jsonl\n         --timeout-ms MS --wait-timeout-ms MS\nAll stdout is JSON. --jsonl emits actual response progress lines.\n--wait polls operation.get; it never resends the original operation.\n--scene/--revision fill missing params.context fields and never replace supplied values.\nMewu must already be running with its CLI plugin enabled.";

#[derive(Clone, Copy, Debug)]
enum Action {
    Help,
    Version,
    Request,
}
#[derive(Clone, Debug)]
struct Invocation {
    action: Action,
    request: Request,
    wait: bool,
    jsonl: bool,
    timeout: Duration,
    wait_timeout: Duration,
}

fn invalid() -> ControlError {
    ControlError::not_sent(
        ErrorCode::InvalidRequest,
        "Invalid command or arguments; run mewu help",
    )
}
fn too_large() -> ControlError {
    ControlError::not_sent(
        ErrorCode::FrameTooLarge,
        "CLI request exceeds the 1 MiB limit",
    )
}
fn unavailable() -> ControlError {
    ControlError::not_sent(
        ErrorCode::Unavailable,
        "Mewu is unavailable; start it normally and enable the CLI plugin",
    )
}
fn unknown(code: ErrorCode) -> ControlError {
    ControlError::unknown(code, "Operation status is unavailable; execution outcome is unknown. Query operation.get using this response id")
}

// Deserialize structured JSON with duplicate-member rejection at every level.
// Value's normal last-member-wins behavior is unsuitable for request context.
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = Unique;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("JSON without duplicate object members")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Unique, E> {
                Ok(Unique(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Unique, E> {
                Ok(Unique(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Unique, E> {
                Number::from_f64(value)
                    .map(|number| Unique(Value::Number(number)))
                    .ok_or_else(|| E::custom("Invalid JSON number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Unique, E> {
                Ok(Unique(Value::String(value.into())))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Unique, E> {
                Ok(Unique(Value::String(value)))
            }
            fn visit_none<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Unique, A::Error> {
                let mut values = Vec::new();
                while let Some(Unique(value)) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Unique, A::Error> {
                let mut values = Map::new();
                while let Some((key, Unique(value))) = object.next_entry::<String, Unique>()? {
                    if values.insert(key, value).is_some() {
                        return Err(de::Error::custom("Duplicate JSON member"));
                    }
                }
                Ok(Unique(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(JsonVisitor)
    }
}

fn bounded_json(reader: impl Read) -> Result<Value, ControlError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_FRAME_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ControlError::not_sent(ErrorCode::InvalidRequest, "Cannot read JSON input"))?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(too_large());
    }
    let Unique(value) = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if !value.is_object() {
        return Err(invalid());
    }
    Ok(value)
}
fn integer(value: &str) -> Result<u64, ControlError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    value.parse().map_err(|_| invalid())
}
fn timeout(value: &str) -> Result<Duration, ControlError> {
    let value = integer(value)?;
    if value == 0 || value > MAX_TIMEOUT_MS {
        return Err(invalid());
    }
    Ok(Duration::from_millis(value))
}
fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn parse(args: Vec<OsString>, stdin: &mut dyn Read) -> Result<Invocation, ControlError> {
    let mut total = 0usize;
    let words = args
        .into_iter()
        .skip(1)
        .map(|argument| {
            let value = argument.into_string().map_err(|_| invalid())?;
            total = total.checked_add(value.len()).ok_or_else(too_large)?;
            if total > MAX_FRAME_BYTES {
                return Err(too_large());
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, ControlError>>()?;
    let mut id = uuid::Uuid::new_v4().to_string();
    let mut input: Option<(&str, String)> = None;
    let (mut wait, mut jsonl) = (false, false);
    let mut request_timeout = Duration::from_secs(10);
    let mut wait_timeout = Duration::from_secs(120);
    let (mut scene, mut revision, mut offset, mut limit) = (None, None, None, None);
    let mut seen = HashSet::new();
    let mut positional = Vec::new();
    let mut index = 0;
    while index < words.len() {
        let word = words[index].as_str();
        if matches!(word, "-h" | "--help") {
            positional.push("help".to_owned());
            index += 1;
            continue;
        }
        if matches!(word, "-V" | "--version") {
            positional.push("version".to_owned());
            index += 1;
            continue;
        }
        if !word.starts_with('-') {
            positional.push(word.to_owned());
            index += 1;
            continue;
        }
        if !matches!(
            word,
            "--request-id"
                | "--json"
                | "--file"
                | "--stdin"
                | "--scene"
                | "--revision"
                | "--wait"
                | "--jsonl"
                | "--timeout-ms"
                | "--wait-timeout-ms"
                | "--offset"
                | "--limit"
        ) || !seen.insert(word.to_owned())
        {
            return Err(invalid());
        }
        if matches!(word, "--wait" | "--jsonl" | "--stdin") {
            match word {
                "--wait" => wait = true,
                "--jsonl" => jsonl = true,
                _ => {
                    if input.replace(("stdin", String::new())).is_some() {
                        return Err(invalid());
                    }
                }
            }
            index += 1;
            continue;
        }
        index += 1;
        let value = words.get(index).ok_or_else(invalid)?;
        match word {
            "--request-id" => id = value.clone(),
            "--json" | "--file" => {
                if input
                    .replace((
                        if word == "--json" { "json" } else { "file" },
                        value.clone(),
                    ))
                    .is_some()
                {
                    return Err(invalid());
                }
            }
            "--scene" => {
                if !identifier(value) {
                    return Err(invalid());
                }
                scene = Some(value.clone());
            }
            "--revision" => {
                let value = integer(value)?;
                if value > SAFE_INTEGER {
                    return Err(invalid());
                }
                revision = Some(value);
            }
            "--offset" => offset = Some(integer(value)?),
            "--limit" => limit = Some(integer(value)?),
            "--timeout-ms" => request_timeout = timeout(value)?,
            "--wait-timeout-ms" => wait_timeout = timeout(value)?,
            _ => return Err(invalid()),
        }
        index += 1;
    }
    let command = positional.iter().map(String::as_str).collect::<Vec<_>>();
    let mut params = json!({});
    let (action, method) = match command.as_slice() {
        [] | ["help"] => (Action::Help, "help".to_owned()),
        ["version"] => (Action::Version, "version".to_owned()),
        ["status"] => (Action::Request, "app.status".to_owned()),
        ["schema"] => (Action::Request, "app.schema".to_owned()),
        ["capabilities"] => (Action::Request, "app.capabilities".to_owned()),
        ["scene", "list"] => {
            if let Some(value) = offset {
                params["offset"] = value.into();
            }
            if let Some(value) = limit {
                params["limit"] = value.into();
            }
            (Action::Request, "scene.list".to_owned())
        }
        ["scene", "get", scene_id] | ["run", "get", scene_id] => {
            if !identifier(scene_id) || offset.is_some() {
                return Err(invalid());
            }
            params["sceneId"] = json!(scene_id);
            if let Some(value) = limit {
                params["limit"] = value.into();
            }
            (
                Action::Request,
                if command[0] == "scene" {
                    "scene.get"
                } else {
                    "run.get"
                }
                .to_owned(),
            )
        }
        ["call", method] => {
            if offset.is_some() || limit.is_some() {
                return Err(invalid());
            }
            params = match input.as_ref() {
                Some(("json", value)) => bounded_json(value.as_bytes())?,
                Some(("file", value)) => {
                    bounded_json(File::open(Path::new(value)).map_err(|_| {
                        ControlError::not_sent(
                            ErrorCode::InvalidRequest,
                            "Cannot read JSON input file",
                        )
                    })?)?
                }
                Some(("stdin", _)) => bounded_json(stdin)?,
                _ => json!({}),
            };
            (Action::Request, (*method).to_owned())
        }
        _ => return Err(invalid()),
    };
    if command.first() != Some(&"call") && input.is_some() {
        return Err(invalid());
    }
    if method != "scene.list"
        && method != "scene.get"
        && method != "run.get"
        && (offset.is_some() || limit.is_some())
    {
        return Err(invalid());
    }
    if !matches!(action, Action::Request)
        && (wait || scene.is_some() || revision.is_some() || seen.contains("--wait-timeout-ms"))
    {
        return Err(invalid());
    }
    if !wait && seen.contains("--wait-timeout-ms") {
        return Err(invalid());
    }
    if scene.is_some() || revision.is_some() {
        let object = params.as_object_mut().ok_or_else(invalid)?;
        let context = object
            .entry("context")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(invalid)?;
        if let Some(value) = scene {
            context.entry("sceneId").or_insert(Value::String(value));
        }
        if let Some(value) = revision {
            context.entry("revision").or_insert(value.into());
        }
    }
    let request = Request::new(id, method, params);
    request.validate()?;
    if serde_json::to_vec(&request).map_err(|_| invalid())?.len() > MAX_FRAME_BYTES {
        return Err(too_large());
    }
    Ok(Invocation {
        action,
        request,
        wait,
        jsonl,
        timeout: request_timeout,
        wait_timeout,
    })
}

trait Control {
    fn call(&mut self, request: &Request, timeout: Duration) -> Result<Response, ControlError>;
}
impl Control for Client {
    fn call(&mut self, request: &Request, timeout: Duration) -> Result<Response, ControlError> {
        Client::call(self, request, timeout)
    }
}
trait Clock {
    fn now(&self) -> Duration;
    fn sleep(&mut self, duration: Duration);
}
struct RealClock(Instant);
impl Clock for RealClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

fn pending_operation(response: &Response) -> Result<Option<String>, ControlError> {
    let Some(result) = response.result() else {
        return Ok(None);
    };
    let Some(operation) = result.get("operationId") else {
        return Ok(None);
    };
    let operation = operation
        .as_str()
        .filter(|value| identifier(value))
        .ok_or_else(|| unknown(ErrorCode::InvalidResponse))?;
    match result.get("status").and_then(Value::as_str) {
        Some("queued" | "accepted" | "running") => Ok(Some(operation.into())),
        Some("completed") if result.get("result").is_some() => Ok(None),
        Some("failed") if result.get("error").is_some() => Ok(None),
        _ => Err(unknown(ErrorCode::InvalidResponse)),
    }
}

// The original call occurs exactly once. Only read-only operation.get requests
// follow an accepted receipt; transport failures never replay an operation.
fn dispatch(
    invocation: &Invocation,
    control: &mut impl Control,
    clock: &mut impl Clock,
    mut emit: impl FnMut(&Response) -> io::Result<()>,
) -> io::Result<Response> {
    let started = clock.now();
    let mut response = control
        .call(&invocation.request, invocation.timeout)
        .unwrap_or_else(|error| Response::error(invocation.request.id.clone(), error));
    if invocation.jsonl {
        emit(&response)?;
    }
    if !invocation.wait || response.error_ref().is_some() {
        return Ok(response);
    }
    let operation = match pending_operation(&response) {
        Ok(Some(operation)) => operation,
        Ok(None) => return Ok(response),
        Err(error) => {
            response = Response::error(invocation.request.id.clone(), error);
            if invocation.jsonl {
                emit(&response)?;
            }
            return Ok(response);
        }
    };
    loop {
        let remaining = invocation
            .wait_timeout
            .saturating_sub(clock.now().saturating_sub(started));
        if remaining.is_zero() {
            response = Response::error(operation.clone(), unknown(ErrorCode::Timeout));
            if invocation.jsonl {
                emit(&response)?;
            }
            return Ok(response);
        }
        clock.sleep(POLL_INTERVAL.min(remaining));
        let remaining = invocation
            .wait_timeout
            .saturating_sub(clock.now().saturating_sub(started));
        if remaining.is_zero() {
            continue;
        }
        let request = Request::new(
            uuid::Uuid::new_v4().to_string(),
            "operation.get",
            json!({"id":operation}),
        );
        response = match control.call(&request, invocation.timeout.min(remaining)) {
            Ok(response) if response.error_ref().is_none() => response,
            Ok(response) => Response::error(
                operation.clone(),
                unknown(response.error_ref().unwrap().code),
            ),
            Err(error) => Response::error(operation.clone(), unknown(error.code)),
        };
        if invocation.jsonl {
            emit(&response)?;
        }
        if response.error_ref().is_some() {
            return Ok(response);
        }
        if response
            .result()
            .and_then(|value| value.get("operationId"))
            .and_then(Value::as_str)
            != Some(operation.as_str())
        {
            response = Response::error(operation.clone(), unknown(ErrorCode::InvalidResponse));
            if invocation.jsonl {
                emit(&response)?;
            }
            return Ok(response);
        }
        match pending_operation(&response) {
            Ok(Some(_)) => (),
            Ok(None) => return Ok(response),
            Err(error) => {
                response = Response::error(operation.clone(), error);
                if invocation.jsonl {
                    emit(&response)?;
                }
                return Ok(response);
            }
        }
    }
}

fn exit_code(response: &Response) -> i32 {
    if let Some(error) = response.error_ref() {
        if error.outcome == ExecutionOutcome::Unknown {
            return 6;
        }
        return match error.code {
            ErrorCode::InvalidRequest | ErrorCode::FrameTooLarge => 2,
            ErrorCode::Unavailable | ErrorCode::Disconnected => 3,
            ErrorCode::PermissionDenied => 4,
            ErrorCode::Timeout => 5,
            _ => 1,
        };
    }
    if response
        .result()
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        == Some("failed")
    {
        return 1;
    }
    0
}
fn stderr_message(code: i32) -> &'static str {
    match code {
        2 => "Invalid CLI input. Run mewu help.",
        3 => "Mewu is unavailable. Start it normally and enable the CLI plugin.",
        4 => "Mewu control identity or permission was rejected.",
        5 => "Mewu request timed out.",
        6 => "Mewu execution outcome is unknown. Do not replay the operation automatically.",
        74 => "Cannot write CLI response.",
        _ => "Mewu operation failed.",
    }
}
fn emit(writer: &mut impl Write, response: &Response) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(response).map_err(io::Error::other)?;
    bytes.push(b'\n');
    writer.write_all(&bytes)?;
    writer.flush()
}
fn companion_server() -> Result<PathBuf, ControlError> {
    let executable = std::env::current_exe().map_err(|_| unavailable())?;
    let directory = executable.parent().ok_or_else(unavailable)?;
    #[cfg(windows)]
    let names = ["MewuAI.exe", "mewu-desktop.exe"];
    #[cfg(not(windows))]
    let names = ["MewuAI", "mewu-desktop"];
    names
        .into_iter()
        .map(|name| directory.join(name))
        .find(|path| path.is_file())
        .ok_or_else(unavailable)
}

/// Root's desktop --cli early branch supplies the known host executable. This
/// does not weaken native peer verification or start a WebView/host process.
pub fn run_for_host(args: Vec<OsString>, server_executable: &Path) -> i32 {
    run_inner(args, Some(server_executable))
}
pub fn run(args: Vec<OsString>) -> i32 {
    run_inner(args, None)
}

fn run_inner(args: Vec<OsString>, server_executable: Option<&Path>) -> i32 {
    let invocation = parse(args, &mut io::stdin().lock());
    let mut stdout = io::stdout().lock();
    let (response, already_emitted) = match invocation {
        Err(error) => (Response::error("cli", error), false),
        Ok(invocation) => match invocation.action {
            Action::Help => (
                Response::success(
                    invocation.request.id,
                    json!({"usage":USAGE,"protocolVersion":PROTOCOL_VERSION}),
                ),
                false,
            ),
            Action::Version => (
                Response::success(
                    invocation.request.id,
                    json!({"version":env!("CARGO_PKG_VERSION"),"protocolVersion":PROTOCOL_VERSION}),
                ),
                false,
            ),
            Action::Request => {
                let connected = (|| {
                    let endpoint = Endpoint::for_current_user().map_err(|error| {
                        if error.kind() == io::ErrorKind::PermissionDenied {
                            ControlError::not_sent(
                                ErrorCode::PermissionDenied,
                                "Local control identity rejected",
                            )
                        } else {
                            unavailable()
                        }
                    })?;
                    // Only the desktop host's explicit --cli entry may opt in.
                    // A failed AppImage proof is final; never retry exact-path
                    // or unauthenticated connections after this selection.
                    #[cfg(target_os = "linux")]
                    if server_executable.is_some() && std::env::var_os("APPIMAGE").is_some() {
                        return Client::connect_appimage_verified(&endpoint, invocation.timeout);
                    }
                    let expected = match server_executable {
                        Some(path) => path.to_path_buf(),
                        None => companion_server()?,
                    };
                    Client::connect_verified(&endpoint, &expected, invocation.timeout)
                })();
                match connected {
                    Err(error) => (Response::error(invocation.request.id, error), false),
                    Ok(mut control) => {
                        let mut clock = RealClock(Instant::now());
                        match dispatch(&invocation, &mut control, &mut clock, |response| {
                            emit(&mut stdout, response)
                        }) {
                            Ok(response) => (response, invocation.jsonl),
                            Err(_) => {
                                eprintln!("{}", stderr_message(74));
                                return 74;
                            }
                        }
                    }
                }
            }
        },
    };
    if !already_emitted && emit(&mut stdout, &response).is_err() {
        eprintln!("{}", stderr_message(74));
        return 74;
    }
    let code = exit_code(&response);
    if code != 0 {
        eprintln!("{}", stderr_message(code));
    }
    code
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
