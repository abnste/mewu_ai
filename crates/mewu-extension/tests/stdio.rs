// SPDX-License-Identifier: MPL-2.0
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

struct Worker {
    child: Child,
    input: ChildStdin,
    output: mpsc::Receiver<Value>,
}
impl Worker {
    fn new() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mewu-extension-worker"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                let value: Value =
                    serde_json::from_str(&line).expect("stdout must be protocol JSON");
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            output,
        }
    }
    fn send(&mut self, value: Value) {
        writeln!(self.input, "{}", serde_json::to_string(&value).unwrap()).unwrap();
        self.input.flush().unwrap();
    }
    fn read(&self) -> Value {
        self.output
            .recv_timeout(Duration::from_secs(5))
            .expect("worker protocol response deadline")
    }
    fn initialize(&mut self, source: &str) {
        self.send(json!({"type":"initialize","id":"init-1","manifest":{"schemaVersion":2,"hostApi":2,"runtime":"javascript","id":"external.stdio","version":"1.0.0","name":"Stdio","description":"","license":"MPL-2.0","entry":"index.js"},"modules":{"index.js":source}}));
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn executable_worker_completes_two_host_calls_and_shutdown_with_no_stdout_noise() {
    let mut worker = Worker::new();
    worker.initialize("export default ctx=>ctx.tools.register({id:'region-agent',title:'Region Agent'},async(input,run)=>{const frame=await ctx.region.observe(run.context.regionId);await ctx.region.action(run.context.regionId,{kind:'click',x:input.x,y:input.y});return {frame,done:true};});");
    let initialized = worker.read();
    assert_eq!(initialized["type"], "initialized");
    assert_eq!(initialized["registrations"][0]["kind"], "tool");
    worker.send(json!({"type":"invoke","id":"run-1","serviceId":"region-agent","input":{"x":12,"y":14},"context":{"regionId":"r-1"}}));
    let call = worker.read();
    assert_eq!(call["type"], "host.call");
    assert_eq!(call["method"], "region.observe");
    assert_eq!(call["params"], json!({"regionId":"r-1"}));
    worker.send(json!({"type":"host.result","id":call["id"],"result":{"type":"image","artifactId":"frame-1"}}));
    let call = worker.read();
    assert_eq!(call["type"], "host.call");
    assert_eq!(call["method"], "region.action");
    assert_eq!(
        call["params"]["action"],
        json!({"kind":"click","x":12,"y":14})
    );
    worker.send(json!({"type":"host.result","id":call["id"],"result":{"executed":true}}));
    let result = worker.read();
    assert_eq!(
        result,
        json!({"type":"result","id":"run-1","result":{"frame":{"type":"image","artifactId":"frame-1"},"done":true}})
    );
    worker.send(json!({"type":"shutdown","id":"stop-1"}));
    assert_eq!(worker.read(), json!({"type":"stopped","id":"stop-1"}));
}

#[test]
fn actual_stdio_cancel_interrupts_cpu_loop_without_killing_host() {
    let mut worker = Worker::new();
    worker.initialize(
        "export default ctx=>ctx.commands.register({id:'loop',title:'Loop'},()=>{while(true){}});",
    );
    assert_eq!(worker.read()["type"], "initialized");
    worker.send(
        json!({"type":"invoke","id":"loop-1","serviceId":"loop","input":null,"context":null}),
    );
    std::thread::sleep(Duration::from_millis(30));
    worker.send(json!({"type":"cancel","id":"loop-1"}));
    let response = worker.read();
    assert_eq!(response["type"], "error");
    assert_eq!(response["error"]["code"], "CANCELED");
}

#[test]
fn errors_are_structured_and_do_not_echo_plugin_thrown_key() {
    let mut worker = Worker::new();
    worker.initialize("export default ctx=>{throw new Error('secret-key-that-must-not-escape')};");
    let response = worker.read();
    assert_eq!(response["type"], "error");
    assert!(!response.to_string().contains("secret-key"));
}

#[test]
fn plugin_activation_can_await_host_storage_before_registering_services() {
    let mut worker = Worker::new();
    worker.initialize("export default async ctx=>{const value=await ctx.storage.get('counter');ctx.memory.register({id:'memory',title:'Memory'},()=>value);};");
    let call = worker.read();
    assert_eq!(call["method"], "storage.get");
    worker.send(json!({"type":"host.result","id":call["id"],"result":{"count":42}}));
    assert_eq!(worker.read()["type"], "initialized");
    worker.send(
        json!({"type":"invoke","id":"read-1","serviceId":"memory","input":null,"context":null}),
    );
    assert_eq!(worker.read()["result"], json!({"count":42}));
}

#[test]
fn duplicate_initialize_does_not_replace_active_plugin() {
    let mut worker = Worker::new();
    worker.initialize("export default ctx=>ctx.commands.register({id:'a',title:'A'},()=>42);");
    assert_eq!(worker.read()["type"], "initialized");
    worker.initialize("export default ctx=>ctx.commands.register({id:'a',title:'A'},()=>99);");
    assert_eq!(worker.read()["error"]["code"], "INVALID_PROTOCOL");
    worker.send(json!({"type":"invoke","id":"read-1","serviceId":"a","input":null,"context":null}));
    assert_eq!(worker.read()["result"], 42);
}
