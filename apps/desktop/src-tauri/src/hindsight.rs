//! Bounded Hindsight HTTP transport; the Store owns all durable work and receipts.
//! Wire authority: Hindsight v0.10.3, eb6df499d35300e5b2f3f029b2e6adda04ed90f8.
//! This adapter has no queue, task spawner, SQLite, credential store, or retry loop.
//! Caller owns immutable route, permission fences, claims, receipts and tombstones.
use futures_util::StreamExt;
use reqwest::{header, Client, Method, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::{sync::oneshot, time::Instant};
use url::Url;

const BODY_LIMIT: usize = 1024 * 1024;
const CONTENT_LIMIT: usize = 64 * 1024;
const REQUEST_LIMIT: usize = 512 * 1024;
const SUPPORTED_VERSION: &str = "0.10.3";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    NotDispatched,
    ReadOnly,
    MutationMayHaveReachedServer,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    InvalidInput,
    Cancelled,
    Deadline,
    Transport,
    Http(u16),
    BodyTooLarge,
    InvalidResponse,
    UnsupportedVersion,
    NotReady,
    IdentityMismatch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdapterError {
    pub code: ErrorCode,
    pub delivery: Delivery,
}
pub type Result<T> = std::result::Result<T, AdapterError>;
fn error(code: ErrorCode, delivery: Delivery) -> AdapterError {
    AdapterError { code, delivery }
}
fn invalid() -> AdapterError {
    error(ErrorCode::InvalidInput, Delivery::NotDispatched)
}

/// Receiver belongs to one ledger dispatch / foreground recall, never to an Agent globally.
pub struct Control<'a> {
    pub deadline: Instant,
    pub cancel: &'a mut oneshot::Receiver<()>,
}

/// Do not derive Debug: reqwest/url objects may include private server information.
pub struct HindsightHttp {
    base: Url,
    client: Client,
}

#[derive(Clone, Debug, Deserialize)]
pub struct VersionInfo {
    pub api_version: String,
    pub features: Features,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Features {
    pub observations: bool,
    pub mcp: bool,
    pub worker: bool,
    pub bank_config_api: bool,
    pub bank_llm_health: bool,
    pub file_upload_api: bool,
    pub document_export_api: bool,
    pub document_import_api: bool,
    pub audit_log: bool,
    pub llm_trace: bool,
    pub store_document_text: bool,
}
#[derive(Deserialize)]
struct HealthWire {
    status: String,
}
#[derive(Deserialize)]
struct ConfigWire {
    bank_id: String,
    config: ResolvedConfig,
    overrides: BTreeMap<String, serde_json::Value>,
}
#[derive(Deserialize)]
struct ResolvedConfig {
    enable_observations: bool,
}
#[derive(Clone, Debug)]
pub struct BankObservation {
    pub bank_id: String,
    pub observations_enabled: bool,
}
#[derive(Clone, Debug)]
pub enum BankLookup {
    Missing,
    Present(BankObservation),
}
#[derive(Deserialize)]
struct BankCreatedWire {
    bank_id: String,
}
#[derive(Serialize)]
struct CreateBankBody {
    enable_observations: bool,
}

/// All IDs and attributed text originate in the Store claim, never in renderer/model arguments.
pub struct RetainInput<'a> {
    pub operation_id: &'a str,
    pub document_id: &'a str,
    pub attributed_text: &'a str,
    pub payload_sha256: &'a str,
}
#[derive(Serialize)]
struct RetainBody<'a> {
    #[serde(rename = "async")]
    asynchronous: bool,
    operation_id: &'a str,
    items: [RetainItem<'a>; 1],
}
#[derive(Serialize)]
struct RetainItem<'a> {
    content: &'a str,
    document_id: &'a str,
    timestamp: &'static str,
    update_mode: &'static str,
    metadata: BTreeMap<&'static str, &'a str>,
}
#[derive(Deserialize)]
struct RetainWire {
    success: bool,
    bank_id: String,
    items_count: u64,
    #[serde(rename = "async")]
    asynchronous: bool,
    operation_id: Option<String>,
    operation_ids: Option<Vec<String>>,
}
#[derive(Clone, Debug)]
pub struct RetainAccepted {
    pub bank_id: String,
    pub operation_id: String,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Pending,
    Processing,
    Completed,
    Failed,
    Cancelled,
    NotFound,
}
#[derive(Deserialize)]
struct OperationWire {
    operation_id: String,
    status: OperationState,
    operation_type: Option<String>,
}
#[derive(Clone, Debug)]
pub struct OperationObservation {
    pub operation_id: String,
    pub state: OperationState,
}
#[derive(Deserialize)]
struct CancelWire {
    success: bool,
    operation_id: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelObservation {
    Acknowledged,
    NotFound,
    AlreadyTerminal,
}

#[derive(Serialize)]
struct RecallBody<'a> {
    query: &'a str,
    types: [&'static str; 2],
    budget: &'static str,
    max_tokens: u32,
    prefer_observations: bool,
    trace: bool,
    include: RecallInclude,
}
#[derive(Serialize)]
struct RecallInclude {
    entities: Option<()>,
    chunks: Option<()>,
    source_facts: Option<()>,
}
#[derive(Deserialize)]
struct RecallWire {
    results: Vec<RecallItemWire>,
}
#[derive(Deserialize)]
struct RecallItemWire {
    id: String,
    text: String,
    document_id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}
#[derive(Clone, Debug)]
pub struct RecallCandidate {
    pub fact_id: String,
    pub text: String,
    pub document_id: String,
    pub kind: String,
}
#[derive(Clone, Debug)]
pub struct RecallCandidates {
    pub candidates: Vec<RecallCandidate>,
    pub rejected_without_provenance: usize,
}
#[derive(Deserialize)]
struct DocumentWire {
    id: String,
    bank_id: String,
    memory_unit_count: u64,
}
#[derive(Clone, Debug)]
pub enum DocumentObservation {
    Missing,
    Present {
        document_id: String,
        memory_unit_count: u64,
    },
}
#[derive(Deserialize)]
struct DeleteWire {
    success: bool,
    document_id: String,
    memory_units_deleted: u64,
}
#[derive(Clone, Debug)]
pub enum DeleteObservation {
    NotFound,
    Deleted {
        document_id: String,
        memory_units_deleted: u64,
    },
}

impl HindsightHttp {
    pub fn new(endpoint: &str) -> Result<Self> {
        let base = validate_endpoint(endpoint)?;
        let client = crate::network_policy::apply(Client::builder(), Some(&base)).map_err(|_| invalid())?
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| invalid())?;
        Ok(Self { base, client })
    }

    pub fn normalized_endpoint(&self) -> String {
        self.base.to_string()
    }

    /// Read-only compatibility/readiness, not an LLM or bank-authorization test.
    /// `features.worker == false` can mean a separately deployed worker; do not reject it.
    pub async fn probe(&self, key: Option<&str>, control: &mut Control<'_>) -> Result<VersionInfo> {
        let version: VersionInfo = self
            .json(Method::GET, &["version"], key, None, false, control)
            .await?;
        if version.api_version != SUPPORTED_VERSION {
            return Err(error(ErrorCode::UnsupportedVersion, Delivery::ReadOnly));
        }
        let ready: HealthWire = self
            .json(Method::GET, &["health", "ready"], key, None, false, control)
            .await?;
        if ready.status != "healthy" {
            return Err(error(ErrorCode::NotReady, Delivery::ReadOnly));
        }
        Ok(version)
    }

    /// Reads do not create a missing bank. Never call retired `/profile`.
    /// Returned bank_id is canonicalized by the release's route class; reject aliases.
    /// Cache/remote-admin changes mean this observation is not a permanent identity guarantee.
    pub async fn inspect_bank(
        &self,
        bank: &str,
        key: Option<&str>,
        control: &mut Control<'_>,
    ) -> Result<BankLookup> {
        let route = bank_route(bank, &["config"])?;
        let response = self
            .request(Method::GET, &route, key, None, false, control)
            .await?;
        if response.status == StatusCode::NOT_FOUND {
            return Ok(BankLookup::Missing);
        }
        let wire: ConfigWire = response.parse()?;
        exact_identity(&wire.bank_id, bank, Delivery::ReadOnly)?;
        // Do not publish resolved config: it can contain private provider/LLM configuration.
        let _ = wire.overrides;
        Ok(BankLookup::Present(BankObservation {
            bank_id: wire.bank_id,
            observations_enabled: wire.config.enable_observations,
        }))
    }

    /// One PUT only. Upstream is UPSERT, not create-if-absent. Host must reserve a fresh
    /// random bank, GET -> 404 first, and refuse collisions. No atomic upstream CAS exists.
    /// Success still requires a subsequent inspect_bank with observations_enabled=false.
    /// On ambiguous failure inspect only; never automatically repeat this PUT.
    pub async fn create_bank_once(
        &self,
        bank: &str,
        key: Option<&str>,
        control: &mut Control<'_>,
    ) -> Result<String> {
        let route = bank_route(bank, &[])?;
        let body = encode(&CreateBankBody {
            enable_observations: false,
        })?;
        let wire: BankCreatedWire = self
            .json(Method::PUT, &route, key, Some(body), true, control)
            .await?;
        exact_identity(&wire.bank_id, bank, Delivery::MutationMayHaveReachedServer)?;
        Ok(wire.bank_id)
    }

    pub async fn retain_once(
        &self,
        bank: &str,
        input: RetainInput<'_>,
        key: Option<&str>,
        control: &mut Control<'_>,
    ) -> Result<RetainAccepted> {
        let route = bank_route(bank, &["memories"])?;
        validate_operation(input.operation_id)?;
        validate_id(input.document_id)?;
        validate_text(input.attributed_text, CONTENT_LIMIT)?;
        if input.payload_sha256.len() != 64
            || !input.payload_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid());
        }
        let body = encode(&RetainBody {
            asynchronous: true,
            operation_id: input.operation_id,
            items: [RetainItem {
                content: input.attributed_text,
                document_id: input.document_id,
                timestamp: "unset",
                update_mode: "replace",
                metadata: BTreeMap::from([("mewu_payload_sha256", input.payload_sha256)]),
            }],
        })?;
        let wire: RetainWire = self
            .json(Method::POST, &route, key, Some(body), true, control)
            .await?;
        validate_retain(wire, bank, input.operation_id)
    }

    pub async fn operation_status(
        &self,
        bank: &str,
        operation: &str,
        key: Option<&str>,
        control: &mut Control<'_>,
    ) -> Result<OperationObservation> {
        validate_operation(operation)?;
        let route = bank_route(bank, &["operations", operation])?;
        let response = self
            .request(Method::GET, &route, key, None, false, control)
            .await?;
        if response.status == StatusCode::NOT_FOUND {
            return Ok(OperationObservation {
                operation_id: operation.into(),
                state: OperationState::NotFound,
            });
        }
        let wire: OperationWire = response.parse()?;
        exact_identity(&wire.operation_id, operation, Delivery::ReadOnly)?;
        if wire
            .operation_type
            .as_deref()
            .is_some_and(|kind| kind != "retain")
        {
            return Err(error(ErrorCode::IdentityMismatch, Delivery::ReadOnly));
        }
        // Cancelled/failed/not_found do not prove no worker will commit a late batch.
        Ok(OperationObservation {
            operation_id: wire.operation_id,
            state: wire.status,
        })
    }

    pub async fn cancel_operation(
        &self,
        bank: &str,
        operation: &str,
        key: Option<&str>,
        control: &mut Control<'_>,
    ) -> Result<CancelObservation> {
        validate_operation(operation)?;
        let route = bank_route(bank, &["operations", operation])?;
        let response = self
            .request(Method::DELETE, &route, key, None, true, control)
            .await?;
        match response.status {
            StatusCode::NOT_FOUND => return Ok(CancelObservation::NotFound),
            StatusCode::CONFLICT => return Ok(CancelObservation::AlreadyTerminal),
            _ => (),
        }
        let wire: CancelWire = response.parse()?;
        exact_identity(
            &wire.operation_id,
            operation,
            Delivery::MutationMayHaveReachedServer,
        )?;
        if !wire.success {
            return Err(error(
                ErrorCode::InvalidResponse,
                Delivery::MutationMayHaveReachedServer,
            ));
        }
        // ACK is not quiescence and not rollback.
        Ok(CancelObservation::Acknowledged)
    }

    pub async fn recall(
        &self,
        bank: &str,
        query: &str,
        key: Option<&str>,
        control: &mut Control<'_>,
    ) -> Result<RecallCandidates> {
        validate_text(query, 8 * 1024)?;
        let route = bank_route(bank, &["memories", "recall"])?;
        let body = encode(&RecallBody {
            query,
            types: ["world", "experience"],
            budget: "low",
            max_tokens: 1500,
            prefer_observations: false,
            trace: false,
            // Omission means default entity expansion upstream. Explicit null disables it.
            include: RecallInclude {
                entities: None,
                chunks: None,
                source_facts: None,
            },
        })?;
        let wire: RecallWire = self
            .json(Method::POST, &route, key, Some(body), false, control)
            .await?;
        validate_recall(wire)
        // Caller MUST intersect these candidates with same-binding local committed evidence
        // and current tombstones BEFORE inserting any text into a model request.
    }

    pub async fn inspect_document(
        &self,
        bank: &str,
        document: &str,
        key: Option<&str>,
        control: &mut Control<'_>,
    ) -> Result<DocumentObservation> {
        validate_id(document)?;
        let route = bank_route(bank, &["documents", document])?;
        let response = self
            .request(Method::GET, &route, key, None, false, control)
            .await?;
        if response.status == StatusCode::NOT_FOUND {
            return Ok(DocumentObservation::Missing);
        }
        let wire: DocumentWire = response.parse()?;
        exact_identity(&wire.bank_id, bank, Delivery::ReadOnly)?;
        exact_identity(&wire.id, document, Delivery::ReadOnly)?;
        // Remote original_text may be null; it is not our local evidence source.
        Ok(DocumentObservation::Present {
            document_id: wire.id,
            memory_unit_count: wire.memory_unit_count,
        })
    }

    pub async fn delete_document_once(
        &self,
        bank: &str,
        document: &str,
        key: Option<&str>,
        control: &mut Control<'_>,
    ) -> Result<DeleteObservation> {
        validate_id(document)?;
        let route = bank_route(bank, &["documents", document])?;
        let response = self
            .request(Method::DELETE, &route, key, None, true, control)
            .await?;
        if response.status == StatusCode::NOT_FOUND {
            return Ok(DeleteObservation::NotFound);
        }
        let wire: DeleteWire = response.parse()?;
        exact_identity(
            &wire.document_id,
            document,
            Delivery::MutationMayHaveReachedServer,
        )?;
        if !wire.success {
            return Err(error(
                ErrorCode::InvalidResponse,
                Delivery::MutationMayHaveReachedServer,
            ));
        }
        Ok(DeleteObservation::Deleted {
            document_id: wire.document_id,
            memory_units_deleted: wire.memory_units_deleted,
        })
        // This only observes deletion at this time. A previously cancelled retain may still write.
    }

    async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        route: &[&str],
        key: Option<&str>,
        body: Option<Vec<u8>>,
        mutation: bool,
        control: &mut Control<'_>,
    ) -> Result<T> {
        self.request(method, route, key, body, mutation, control)
            .await?
            .parse()
    }
    async fn request(
        &self,
        method: Method,
        route: &[&str],
        key: Option<&str>,
        body: Option<Vec<u8>>,
        mutation: bool,
        control: &mut Control<'_>,
    ) -> Result<ResponseBytes> {
        let url = endpoint_path(&self.base, route)?;
        let mut builder = self
            .client
            .request(method, url)
            .header(header::ACCEPT, "application/json");
        if let Some(key) = key {
            if key.is_empty() || key.len() > 16 * 1024 || key.chars().any(char::is_control) {
                return Err(invalid());
            }
            let mut value =
                header::HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| invalid())?;
            value.set_sensitive(true);
            builder = builder.header(header::AUTHORIZATION, value);
        }
        if let Some(body) = body {
            if body.len() > REQUEST_LIMIT {
                return Err(invalid());
            }
            builder = builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        let request = builder.build().map_err(|_| invalid())?;
        let dispatched = AtomicBool::new(false);
        let exchange = async {
            dispatched.store(true, Ordering::Relaxed);
            let response = self
                .client
                .execute(request)
                .await
                .map_err(|_| ErrorCode::Transport)?;
            let status = response.status();
            if response
                .content_length()
                .is_some_and(|n| n > BODY_LIMIT as u64)
            {
                return Err(ErrorCode::BodyTooLarge);
            }
            let mut bytes = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| ErrorCode::Transport)?;
                if chunk.len() > BODY_LIMIT - bytes.len() {
                    return Err(ErrorCode::BodyTooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok((status, bytes))
        };
        let result = tokio::select! {
            biased;
            _ = &mut *control.cancel => Err(ErrorCode::Cancelled),
            _ = tokio::time::sleep_until(control.deadline) => Err(ErrorCode::Deadline),
            result = exchange => result,
        };
        let delivery = if !dispatched.load(Ordering::Relaxed) {
            Delivery::NotDispatched
        } else if mutation {
            Delivery::MutationMayHaveReachedServer
        } else {
            Delivery::ReadOnly
        };
        result
            .map(|(status, bytes)| ResponseBytes {
                status,
                bytes,
                delivery,
            })
            .map_err(|code| error(code, delivery))
    }
}

struct ResponseBytes {
    status: StatusCode,
    bytes: Vec<u8>,
    delivery: Delivery,
}
impl ResponseBytes {
    fn parse<T: DeserializeOwned>(self) -> Result<T> {
        if self.status != StatusCode::OK {
            return Err(error(ErrorCode::Http(self.status.as_u16()), self.delivery));
        }
        serde_json::from_slice(&self.bytes)
            .map_err(|_| error(ErrorCode::InvalidResponse, self.delivery))
    }
}
fn encode<T: Serialize>(input: &T) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(input).map_err(|_| invalid())?;
    if bytes.len() > REQUEST_LIMIT {
        return Err(invalid());
    }
    Ok(bytes)
}
fn validate_endpoint(endpoint: &str) -> Result<Url> {
    if endpoint.len() > 2048 || endpoint.chars().any(char::is_control) {
        return Err(invalid());
    }
    let url = Url::parse(endpoint).map_err(|_| invalid())?;
    let loopback = match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || url.scheme() == "http" && loopback)
    {
        return Err(invalid());
    }
    Ok(url)
}
fn endpoint_path(base: &Url, segments: &[&str]) -> Result<Url> {
    let mut url = base.clone();
    {
        let mut path = url.path_segments_mut().map_err(|_| invalid())?;
        path.pop_if_empty();
        for segment in segments {
            path.push(segment);
        }
    }
    Ok(url)
}
fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(invalid());
    }
    Ok(())
}
fn validate_operation(id: &str) -> Result<()> {
    let uuid = uuid::Uuid::parse_str(id).map_err(|_| invalid())?;
    if uuid.hyphenated().to_string() != id {
        return Err(invalid());
    }
    Ok(())
}
fn validate_text(text: &str, max: usize) -> Result<()> {
    if text.trim().is_empty()
        || text.len() > max
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(invalid());
    }
    Ok(())
}
fn bank_route<'a>(bank: &'a str, tail: &[&'a str]) -> Result<Vec<&'a str>> {
    validate_id(bank)?;
    let mut route = vec!["v1", "default", "banks", bank];
    route.extend_from_slice(tail);
    Ok(route)
}
fn exact_identity(actual: &str, expected: &str, delivery: Delivery) -> Result<()> {
    if actual != expected {
        Err(error(ErrorCode::IdentityMismatch, delivery))
    } else {
        Ok(())
    }
}
fn validate_retain(wire: RetainWire, bank: &str, operation: &str) -> Result<RetainAccepted> {
    let delivery = Delivery::MutationMayHaveReachedServer;
    exact_identity(&wire.bank_id, bank, delivery)?;
    if !wire.success
        || !wire.asynchronous
        || wire.items_count != 1
        || wire.operation_id.as_deref() != Some(operation)
        || wire
            .operation_ids
            .as_ref()
            .is_some_and(|ids| ids.len() != 1 || ids[0] != operation)
    {
        return Err(error(ErrorCode::InvalidResponse, delivery));
    }
    Ok(RetainAccepted {
        bank_id: wire.bank_id,
        operation_id: operation.into(),
    })
}
fn validate_recall(wire: RecallWire) -> Result<RecallCandidates> {
    let bad = || error(ErrorCode::InvalidResponse, Delivery::ReadOnly);
    if wire.results.len() > 128 {
        return Err(bad());
    }
    let mut candidates = Vec::new();
    let mut rejected = 0;
    for item in wire.results {
        let (Some(document_id), Some(kind)) = (item.document_id, item.kind) else {
            rejected += 1;
            continue;
        };
        if !matches!(kind.as_str(), "world" | "experience") || validate_id(&document_id).is_err() {
            rejected += 1;
            continue;
        }
        if item.id.is_empty()
            || item.id.len() > 256
            || item.id.chars().any(char::is_control)
            || validate_text(&item.text, 16 * 1024).is_err()
        {
            return Err(bad());
        }
        candidates.push(RecallCandidate {
            fact_id: item.id,
            text: item.text,
            document_id,
            kind,
        });
    }
    Ok(RecallCandidates {
        candidates,
        rejected_without_provenance: rejected,
    })
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use tokio::sync::mpsc;

    const OP: &str = "66895537-68e5-4539-9984-5b4b1b4d838b";
    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    #[derive(Debug)]
    struct Captured {
        method: String,
        path: String,
        headers: BTreeMap<String, String>,
        body: Vec<u8>,
    }
    struct Fixture {
        endpoint: String,
        requests: mpsc::UnboundedReceiver<Captured>,
        thread: thread::JoinHandle<()>,
    }
    fn capture(mut stream: &TcpStream) -> Captured {
        // Windows accepted sockets inherit the listener's nonblocking mode.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 2048];
        let header_end = loop {
            let n = stream.read(&mut buffer).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buffer[..n]);
            assert!(bytes.len() <= REQUEST_LIMIT + 32 * 1024);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                break end + 4;
            }
        };
        let head = std::str::from_utf8(&bytes[..header_end]).unwrap();
        let mut lines = head.lines();
        let first = lines.next().unwrap();
        let mut first = first.split_whitespace();
        let method = first.next().unwrap().to_owned();
        let path = first.next().unwrap().to_owned();
        let headers: BTreeMap<String, String> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        let length: usize = headers
            .get("content-length")
            .map(|v| v.parse().unwrap())
            .unwrap_or(0);
        assert!(length <= REQUEST_LIMIT);
        while bytes.len() - header_end < length {
            let n = stream.read(&mut buffer).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buffer[..n]);
        }
        Captured {
            method,
            path,
            headers,
            body: bytes[header_end..header_end + length].to_vec(),
        }
    }
    fn fixture(responses: Vec<Vec<u8>>) -> Fixture {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/prefix", listener.local_addr().unwrap());
        let (sender, requests) = mpsc::unbounded_channel();
        let thread = thread::spawn(move || {
            for response in responses {
                let until = std::time::Instant::now() + Duration::from_secs(3);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < until,
                                "fixture request did not arrive"
                            );
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(e) => panic!("fixture accept failed: {e}"),
                    }
                };
                stream
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                sender.send(capture(&stream)).unwrap();
                // An empty response is a deterministic stalled server, used only in cancellation tests.
                if response.is_empty() {
                    thread::sleep(Duration::from_millis(250));
                } else {
                    let _ = stream.write_all(&response);
                }
            }
        });
        Fixture {
            endpoint,
            requests,
            thread,
        }
    }
    fn response(status: u16, value: serde_json::Value) -> Vec<u8> {
        let body = serde_json::to_vec(&value).unwrap();
        let mut out=format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",body.len()).into_bytes();
        out.extend_from_slice(&body);
        out
    }
    fn version() -> serde_json::Value {
        serde_json::json!({"api_version":"0.10.3","features":{
            "observations":true,"mcp":false,"worker":false,"bank_config_api":false,
            "bank_llm_health":false,"file_upload_api":false,"document_export_api":false,
            "document_import_api":false,"audit_log":false,"llm_trace":false,"store_document_text":false
        }})
    }
    fn config(bank: &str) -> serde_json::Value {
        serde_json::json!({"bank_id":bank,"config":{"enable_observations":false,"private_other_field":"not returned"},"overrides":{}})
    }
    fn retain() -> RetainInput<'static> {
        RetainInput {
            operation_id: OP,
            document_id: "document",
            attributed_text: "User said: synthetic evidence",
            payload_sha256: HASH,
        }
    }
    fn control(cancel: &mut oneshot::Receiver<()>) -> Control<'_> {
        Control {
            deadline: Instant::now() + Duration::from_secs(3),
            cancel,
        }
    }
    #[tokio::test(flavor = "current_thread")]
    async fn setup_http_uses_config_and_does_not_gate_read_on_global_flags() {
        let mut f = fixture(vec![
            response(200, version()),
            response(200, serde_json::json!({"status":"healthy"})),
            response(404, serde_json::json!({})),
            response(200, serde_json::json!({"bank_id":"bank"})),
            response(200, config("bank")),
        ]);
        let api = HindsightHttp::new(&f.endpoint).unwrap();
        let (_sender, mut cancel) = oneshot::channel();
        let mut c = control(&mut cancel);
        let version = api.probe(Some("fixture-only"), &mut c).await.unwrap();
        assert!(!version.features.worker);
        assert!(matches!(
            api.inspect_bank("bank", Some("fixture-only"), &mut c)
                .await
                .unwrap(),
            BankLookup::Missing
        ));
        api.create_bank_once("bank", Some("fixture-only"), &mut c)
            .await
            .unwrap();
        let BankLookup::Present(bank) = api
            .inspect_bank("bank", Some("fixture-only"), &mut c)
            .await
            .unwrap()
        else {
            panic!()
        };
        assert!(!bank.observations_enabled);
        for expected in [
            "/prefix/version",
            "/prefix/health/ready",
            "/prefix/v1/default/banks/bank/config",
            "/prefix/v1/default/banks/bank",
            "/prefix/v1/default/banks/bank/config",
        ] {
            let request = f.requests.recv().await.unwrap();
            assert_eq!(request.path, expected);
            assert_eq!(
                request.headers.get("authorization").unwrap(),
                "Bearer fixture-only"
            );
            if request.method == "PUT" {
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&request.body).unwrap(),
                    serde_json::json!({"enable_observations":false})
                );
            }
        }
        f.thread.join().unwrap();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn retain_and_poll_are_distinct_and_ids_are_exact() {
        let mut replies = vec![response(
            200,
            serde_json::json!({"success":true,"bank_id":"bank","items_count":1,"async":true,"operation_id":OP}),
        )];
        for status in [
            "pending",
            "processing",
            "completed",
            "failed",
            "cancelled",
            "not_found",
        ] {
            replies.push(response(
                200,
                serde_json::json!({"operation_id":OP,"status":status,"operation_type":"retain"}),
            ));
        }
        let mut f = fixture(replies);
        let api = HindsightHttp::new(&f.endpoint).unwrap();
        let (_sender, mut cancel) = oneshot::channel();
        let mut c = control(&mut cancel);
        assert_eq!(
            api.retain_once("bank", retain(), None, &mut c)
                .await
                .unwrap()
                .operation_id,
            OP
        );
        let request = f.requests.recv().await.unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/prefix/v1/default/banks/bank/memories");
        assert!(!request.headers.contains_key("authorization"));
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["operation_id"], OP);
        assert_eq!(body["async"], true);
        assert_eq!(body["items"][0]["timestamp"], "unset");
        assert_eq!(body["items"][0]["document_id"], "document");
        assert_eq!(body["items"][0]["metadata"]["mewu_payload_sha256"], HASH);
        for state in [
            OperationState::Pending,
            OperationState::Processing,
            OperationState::Completed,
            OperationState::Failed,
            OperationState::Cancelled,
            OperationState::NotFound,
        ] {
            assert_eq!(
                api.operation_status("bank", OP, None, &mut c)
                    .await
                    .unwrap()
                    .state,
                state
            );
            assert!(!f
                .requests
                .recv()
                .await
                .unwrap()
                .path
                .contains("include_payload"));
        }
        f.thread.join().unwrap();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn cancel_and_document_responses_preserve_uncertainty() {
        let f = fixture(vec![
            response(200, serde_json::json!({"success":true,"operation_id":OP})),
            response(404, serde_json::json!({})),
            response(409, serde_json::json!({})),
            response(
                200,
                serde_json::json!({"id":"document","bank_id":"bank","original_text":null,"memory_unit_count":2}),
            ),
            response(
                200,
                serde_json::json!({"success":true,"document_id":"document","memory_units_deleted":2}),
            ),
            response(404, serde_json::json!({})),
        ]);
        let api = HindsightHttp::new(&f.endpoint).unwrap();
        let (_sender, mut cancel) = oneshot::channel();
        let mut c = control(&mut cancel);
        for value in [
            CancelObservation::Acknowledged,
            CancelObservation::NotFound,
            CancelObservation::AlreadyTerminal,
        ] {
            assert_eq!(
                api.cancel_operation("bank", OP, None, &mut c)
                    .await
                    .unwrap(),
                value
            );
        }
        assert!(matches!(
            api.inspect_document("bank", "document", None, &mut c)
                .await
                .unwrap(),
            DocumentObservation::Present {
                memory_unit_count: 2,
                ..
            }
        ));
        assert!(matches!(
            api.delete_document_once("bank", "document", None, &mut c)
                .await
                .unwrap(),
            DeleteObservation::Deleted {
                memory_units_deleted: 2,
                ..
            }
        ));
        assert!(matches!(
            api.delete_document_once("bank", "document", None, &mut c)
                .await
                .unwrap(),
            DeleteObservation::NotFound
        ));
        f.thread.join().unwrap();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn recall_is_bounded_provenance_candidates_and_has_explicit_null_expansions() {
        let mut f = fixture(vec![response(
            200,
            serde_json::json!({"results":[
            {"id":"fact1","text":"candidate","type":"world","document_id":"document"},
            {"id":"fact2","text":"no provenance","type":"experience"}]}),
        )]);
        let api = HindsightHttp::new(&f.endpoint).unwrap();
        let (_sender, mut cancel) = oneshot::channel();
        let result = api
            .recall("bank", "synthetic query", None, &mut control(&mut cancel))
            .await
            .unwrap();
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.rejected_without_provenance, 1);
        let request = f.requests.recv().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body["include"],
            serde_json::json!({"entities":null,"chunks":null,"source_facts":null})
        );
        assert_eq!(body["types"], serde_json::json!(["world", "experience"]));
        assert_eq!(body["max_tokens"], 1500);
        f.thread.join().unwrap();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn canonical_alias_and_mutation_receipt_mismatch_are_rejected() {
        let f = fixture(vec![
            response(200, config("otherbank")),
            response(
                200,
                serde_json::json!({"success":true,"bank_id":"bank","items_count":1,"async":true,"operation_id":"different"}),
            ),
        ]);
        let api = HindsightHttp::new(&f.endpoint).unwrap();
        let (_sender, mut cancel) = oneshot::channel();
        let mut c = control(&mut cancel);
        assert_eq!(
            api.inspect_bank("bank", None, &mut c)
                .await
                .unwrap_err()
                .code,
            ErrorCode::IdentityMismatch
        );
        assert_eq!(
            api.retain_once("bank", retain(), None, &mut c)
                .await
                .unwrap_err()
                .delivery,
            Delivery::MutationMayHaveReachedServer
        );
        f.thread.join().unwrap();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn cancellation_before_dispatch_sends_nothing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let api =
            HindsightHttp::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let (sender, mut cancel) = oneshot::channel();
        sender.send(()).unwrap();
        let result = api
            .retain_once("bank", retain(), None, &mut control(&mut cancel))
            .await
            .unwrap_err();
        assert_eq!(result, error(ErrorCode::Cancelled, Delivery::NotDispatched));
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    #[tokio::test(flavor = "current_thread")]
    async fn cancellation_and_timeout_after_dispatch_are_unknown_mutations() {
        for cancel_request in [true, false] {
            let mut f = fixture(vec![Vec::new()]);
            let api = HindsightHttp::new(&f.endpoint).unwrap();
            let (sender, mut cancel) = oneshot::channel();
            let request = tokio::spawn(async move {
                api.retain_once(
                    "bank",
                    retain(),
                    None,
                    &mut Control {
                        deadline: Instant::now() + Duration::from_millis(150),
                        cancel: &mut cancel,
                    },
                )
                .await
            });
            f.requests.recv().await.unwrap();
            if cancel_request {
                sender.send(()).unwrap();
            } else {
                let _keep_alive = sender;
                assert_eq!(
                    request.await.unwrap().unwrap_err(),
                    error(ErrorCode::Deadline, Delivery::MutationMayHaveReachedServer)
                );
                f.thread.join().unwrap();
                continue;
            }
            assert_eq!(
                request.await.unwrap().unwrap_err(),
                error(ErrorCode::Cancelled, Delivery::MutationMayHaveReachedServer)
            );
            f.thread.join().unwrap();
        }
    }
    #[tokio::test(flavor = "current_thread")]
    async fn redirect_and_service_error_never_reissue_request() {
        let destination = TcpListener::bind("127.0.0.1:0").unwrap();
        destination.set_nonblocking(true).unwrap();
        let redirect=format!("HTTP/1.1 302 Found\r\nLocation: http://{}/credential-target\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",destination.local_addr().unwrap()).into_bytes();
        let mut f = fixture(vec![
            redirect,
            response(503, serde_json::json!({"detail":"fixture failure"})),
        ]);
        let api = HindsightHttp::new(&f.endpoint).unwrap();
        let (_sender, mut cancel) = oneshot::channel();
        let mut c = control(&mut cancel);
        assert_eq!(
            api.inspect_bank("bank", Some("fixture-only"), &mut c)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Http(302)
        );
        assert_eq!(
            destination.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(
            api.retain_once("bank", retain(), None, &mut c)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Http(503)
        );
        f.thread.join().unwrap();
        assert!(f.requests.recv().await.is_some());
        assert!(f.requests.recv().await.is_some());
        assert!(f.requests.recv().await.is_none());
    }
    #[tokio::test(flavor = "current_thread")]
    async fn chunked_body_budget_and_truncated_response_are_errors() {
        let bytes = vec![b'x'; BODY_LIMIT + 1];
        let mut chunked = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n",
            bytes.len()
        )
        .into_bytes();
        chunked.extend_from_slice(&bytes);
        chunked.extend_from_slice(b"\r\n0\r\n\r\n");
        let truncated =
            b"HTTP/1.1 200 OK\r\nContent-Length: 200\r\nConnection: close\r\n\r\n{}".to_vec();
        let f = fixture(vec![chunked, truncated]);
        let api = HindsightHttp::new(&f.endpoint).unwrap();
        let (_sender, mut cancel) = oneshot::channel();
        let mut c = control(&mut cancel);
        assert_eq!(
            api.inspect_bank("bank", None, &mut c)
                .await
                .unwrap_err()
                .code,
            ErrorCode::BodyTooLarge
        );
        assert_eq!(
            api.retain_once("bank", retain(), None, &mut c)
                .await
                .unwrap_err()
                .delivery,
            Delivery::MutationMayHaveReachedServer
        );
        f.thread.join().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const OP: &str = "66895537-68e5-4539-9984-5b4b1b4d838b";
    #[test]
    fn fixed_paths_preserve_prefix_and_encode_segments() {
        let base = validate_endpoint("https://example.test/hindsight/").unwrap();
        assert_eq!(
            endpoint_path(&base, &["v1", "default", "banks", "a/b?#"])
                .unwrap()
                .as_str(),
            "https://example.test/hindsight/v1/default/banks/a%2Fb%3F%23"
        );
        assert!(validate_id("a/b").is_err());
        assert!(validate_id("..").is_err());
    }
    #[test]
    fn endpoint_and_operation_reject_untrusted_shape() {
        for value in [
            "http://remote.test",
            "https://u:p@example.test",
            "https://example.test/?key=x",
            "file:///tmp/a",
        ] {
            assert!(validate_endpoint(value).is_err());
        }
        assert!(validate_endpoint("http://127.0.0.1:8888").is_ok());
        assert!(validate_endpoint("http://[::1]:8888").is_ok());
        assert!(validate_operation(OP).is_ok());
        assert!(validate_operation("../x").is_err());
    }
    #[test]
    fn duplicate_known_fields_and_unknown_status_are_rejected() {
        assert!(serde_json::from_str::<OperationWire>(
            r#"{"operation_id":"a","operation_id":"b","status":"completed"}"#
        )
        .is_err());
        assert!(serde_json::from_str::<OperationWire>(
            r#"{"operation_id":"a","status":"succeeded"}"#
        )
        .is_err());
    }
    #[test]
    fn accepted_is_exact_one_async_operation() {
        let make = |bank: &str, count: u64, asynchronous: bool| RetainWire {
            success: true,
            bank_id: bank.into(),
            items_count: count,
            asynchronous,
            operation_id: Some(OP.into()),
            operation_ids: None,
        };
        assert!(validate_retain(make("bank", 1, true), "bank", OP).is_ok());
        assert!(validate_retain(make("other", 1, true), "bank", OP).is_err());
        assert!(validate_retain(make("bank", 2, true), "bank", OP).is_err());
        assert!(validate_retain(make("bank", 1, false), "bank", OP).is_err());
    }
    #[test]
    fn recall_without_owned_document_provenance_is_not_candidate() {
        let wire: RecallWire = serde_json::from_str(
            r#"{"results":[
            {"id":"a","text":"no doc","type":"world"},
            {"id":"b","text":"derived","type":"observation","document_id":"d"},
            {"id":"c","text":"candidate only","type":"experience","document_id":"d"}] }"#,
        )
        .unwrap();
        let result = validate_recall(wire).unwrap();
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.rejected_without_provenance, 2);
    }
    #[test]
    fn explicit_null_disables_default_entity_expansion() {
        let value = serde_json::to_value(RecallInclude {
            entities: None,
            chunks: None,
            source_facts: None,
        })
        .unwrap();
        assert!(value.get("entities").unwrap().is_null());
    }
    #[test]
    fn required_resolved_observation_flag_is_not_guessed_from_version() {
        assert!(serde_json::from_str::<ConfigWire>(
            r#"{"bank_id":"b","config":{},"overrides":{}}"#
        )
        .is_err());
        let config: ConfigWire = serde_json::from_str(
            r#"{"bank_id":"b","config":{"enable_observations":false},"overrides":{}}"#,
        )
        .unwrap();
        assert!(!config.config.enable_observations);
    }
}
