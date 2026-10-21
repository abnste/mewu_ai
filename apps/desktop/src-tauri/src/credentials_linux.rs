// SPDX-License-Identifier: MPL-2.0
//! Immutable credentials in the desktop user's existing Secret Service.
//! No shell, plaintext file fallback, service activation, collection creation or
//! unlock prompt. A locked/missing keyring remains an actionable failure.

use serde::{de::DeserializeOwned, Serialize};
use std::{
    collections::HashMap,
    future::Future,
    time::{Duration, Instant},
};
use zbus::{
    zvariant::{DynamicType, OwnedObjectPath, OwnedValue, Type, Value},
    Connection,
};
use zeroize::Zeroizing;

const SERVICE: &str = "org.freedesktop.secrets";
const ROOT: &str = "/org/freedesktop/secrets";
const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const CONTENT_TYPE: &str = "text/plain; charset=utf-8";
const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REPLY: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Unavailable,
    Locked,
    Invalid,
    Timeout,
}
impl Error {
    fn message(self) -> String {
        match self {
            Self::Unavailable => "无法访问桌面密钥环，请启用当前用户的 Secret Service",
            Self::Locked => "桌面密钥环已锁定，请解锁后重试",
            Self::Invalid => "桌面密钥环中的连接凭据无效",
            Self::Timeout => "访问桌面密钥环超时，请重试",
        }
        .into()
    }
}
type Result<T> = std::result::Result<T, Error>;
type Attributes = HashMap<String, String>;
type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

fn attributes(namespace: &str, owner: &str, credential: &str) -> Result<Attributes> {
    if !matches!(namespace, "connections" | "memory" | "extensions")
        || super::validate_id(owner).is_err()
        || super::validate_id(credential).is_err()
    {
        return Err(Error::Invalid);
    }
    Ok([
        ("application", "dev.mewu.remake"),
        ("schema", "immutable-v1"),
        ("namespace", namespace),
        ("owner", owner),
        ("credential", credential),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect())
}

async fn bounded<T>(deadline: Instant, task: impl Future<Output = Result<T>>) -> Result<T> {
    if Instant::now() >= deadline {
        return Err(Error::Timeout);
    }
    futures_lite::future::race(task, async {
        async_io::Timer::at(deadline).await;
        Err(Error::Timeout)
    })
    .await
}

async fn call<B, R>(
    conn: &Connection,
    destination: &str,
    path: &str,
    interface: &str,
    method: &str,
    body: &B,
) -> Result<R>
where
    B: Serialize + DynamicType,
    R: DeserializeOwned + Type,
{
    let reply = conn
        .call_method(Some(destination), path, Some(interface), method, body)
        .await
        .map_err(|_| Error::Unavailable)?;
    // Bound decoding before allocating maps, item arrays or secret byte vectors.
    if reply.data().bytes().len() > MAX_REPLY {
        return Err(Error::Invalid);
    }
    reply.body().deserialize().map_err(|_| Error::Invalid)
}

async fn property<T>(
    conn: &Connection,
    owner: &str,
    path: &str,
    interface: &str,
    property: &str,
) -> Result<T>
where
    T: TryFrom<OwnedValue>,
    T::Error: std::fmt::Debug,
{
    let (value,): (OwnedValue,) = call(
        conn,
        owner,
        path,
        "org.freedesktop.DBus.Properties",
        "Get",
        &(interface, property),
    )
    .await?;
    T::try_from(value).map_err(|_| Error::Invalid)
}

fn path_valid(path: &OwnedObjectPath) -> Result<()> {
    if path.as_str() == "/" || path.as_str().len() > 2048 {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}

async fn service_owner(conn: &Connection) -> Result<String> {
    // Do not activate an absent daemon. Pin its unique bus owner for all further
    // calls: replacement of the well-known name must never redirect a secret.
    let (owner,): (String,) = call(conn, BUS, BUS_PATH, BUS, "GetNameOwner", &(SERVICE,)).await?;
    if !owner.starts_with(':') || owner.len() > 255 {
        return Err(Error::Invalid);
    }
    let (uid,): (u32,) = call(
        conn,
        BUS,
        BUS_PATH,
        BUS,
        "GetConnectionUnixUser",
        &(owner.as_str(),),
    )
    .await?;
    if uid != rustix::process::geteuid().as_raw() {
        return Err(Error::Invalid);
    }
    Ok(owner)
}

async fn session(conn: &Connection, owner: &str) -> Result<OwnedObjectPath> {
    // The standard plain session uses the authenticated local user bus. The
    // keyring owns storage encryption; lookup attributes never contain a key.
    let (output, session): (OwnedValue, OwnedObjectPath) = call(
        conn,
        owner,
        ROOT,
        "org.freedesktop.Secret.Service",
        "OpenSession",
        &("plain", Value::from("")),
    )
    .await?;
    if String::try_from(output).map_err(|_| Error::Invalid)? != "" {
        return Err(Error::Invalid);
    }
    path_valid(&session)?;
    Ok(session)
}

async fn find(
    conn: &Connection,
    owner: &str,
    attrs: &Attributes,
) -> Result<Option<OwnedObjectPath>> {
    let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = call(
        conn,
        owner,
        ROOT,
        "org.freedesktop.Secret.Service",
        "SearchItems",
        &(attrs,),
    )
    .await?;
    if !locked.is_empty() {
        return Err(Error::Locked);
    }
    match unlocked.as_slice() {
        [] => Ok(None),
        [item] => {
            path_valid(item)?;
            verify_attributes(conn, owner, item, attrs).await?;
            Ok(Some(item.clone()))
        }
        _ => Err(Error::Invalid),
    }
}

async fn verify_attributes(
    conn: &Connection,
    owner: &str,
    item: &OwnedObjectPath,
    attrs: &Attributes,
) -> Result<()> {
    let found: Attributes = property(
        conn,
        owner,
        item.as_str(),
        "org.freedesktop.Secret.Item",
        "Attributes",
    )
    .await?;
    if &found != attrs {
        return Err(Error::Invalid);
    }
    let locked: bool = property(
        conn,
        owner,
        item.as_str(),
        "org.freedesktop.Secret.Item",
        "Locked",
    )
    .await?;
    if locked {
        return Err(Error::Locked);
    }
    Ok(())
}

async fn read_item(
    conn: &Connection,
    owner: &str,
    item: &OwnedObjectPath,
    session: &OwnedObjectPath,
) -> Result<Zeroizing<String>> {
    let (secret,): (Secret,) = call(
        conn,
        owner,
        item.as_str(),
        "org.freedesktop.Secret.Item",
        "GetSecret",
        &(session,),
    )
    .await?;
    let bytes = Zeroizing::new(secret.2);
    if secret.0 != *session
        || !secret.1.is_empty()
        || secret.3 != CONTENT_TYPE
        || bytes.len() > 16 * 1024
    {
        return Err(Error::Invalid);
    }
    let key = Zeroizing::new(
        std::str::from_utf8(&bytes)
            .map_err(|_| Error::Invalid)?
            .to_owned(),
    );
    if key.is_empty() || super::validate_key(&key).is_err() {
        return Err(Error::Invalid);
    }
    Ok(key)
}

enum Operation<'a> {
    Read,
    Create(&'a str),
    Discard,
}

async fn operation(
    conn: &Connection,
    attrs: &Attributes,
    action: Operation<'_>,
) -> Result<Option<Zeroizing<String>>> {
    let owner = service_owner(conn).await?;
    let existing = find(conn, &owner, attrs).await?;
    match action {
        Operation::Read => {
            let item = existing.ok_or(Error::Invalid)?;
            let session = session(conn, &owner).await?;
            read_item(conn, &owner, &item, &session).await.map(Some)
        }
        Operation::Create(key) => {
            if existing.is_some() {
                return Err(Error::Invalid);
            }
            let (collection,): (OwnedObjectPath,) = call(
                conn,
                &owner,
                ROOT,
                "org.freedesktop.Secret.Service",
                "ReadAlias",
                &("default",),
            )
            .await?;
            path_valid(&collection)?;
            if property::<bool>(
                conn,
                &owner,
                collection.as_str(),
                "org.freedesktop.Secret.Collection",
                "Locked",
            )
            .await?
            {
                return Err(Error::Locked);
            }
            let session = session(conn, &owner).await?;
            let properties = HashMap::from([
                (
                    "org.freedesktop.Secret.Item.Label",
                    Value::from("Mewu connection credential"),
                ),
                (
                    "org.freedesktop.Secret.Item.Attributes",
                    Value::new(attrs.clone()),
                ),
            ]);
            let secret = (&session, &[] as &[u8], key.as_bytes(), CONTENT_TYPE);
            let (item, prompt): (OwnedObjectPath, OwnedObjectPath) = call(
                conn,
                &owner,
                collection.as_str(),
                "org.freedesktop.Secret.Collection",
                "CreateItem",
                &(properties, secret, false),
            )
            .await?;
            // replace=false preserves every existing immutable credential.
            // Interactive collection/unlock setup belongs to the desktop user.
            if prompt.as_str() != "/" {
                return Err(Error::Locked);
            }
            path_valid(&item)?;
            verify_attributes(conn, &owner, &item, attrs).await?;
            let saved = read_item(conn, &owner, &item, &session).await?;
            if saved.as_str() != key {
                return Err(Error::Invalid);
            }
            Ok(None)
        }
        Operation::Discard => {
            if let Some(item) = existing {
                let (prompt,): (OwnedObjectPath,) = call(
                    conn,
                    &owner,
                    item.as_str(),
                    "org.freedesktop.Secret.Item",
                    "Delete",
                    &(),
                )
                .await?;
                if prompt.as_str() != "/" {
                    return Err(Error::Locked);
                }
            }
            Ok(None)
        }
    }
}

fn run(
    namespace: &str,
    owner: &str,
    credential: &str,
    action: Operation<'_>,
) -> Result<Option<Zeroizing<String>>> {
    let address = zbus::Address::session().map_err(|_| Error::Unavailable)?;
    run_on(address, TIMEOUT, namespace, owner, credential, action)
}

fn run_on(
    address: zbus::Address,
    timeout: Duration,
    namespace: &str,
    owner: &str,
    credential: &str,
    action: Operation<'_>,
) -> Result<Option<Zeroizing<String>>> {
    let attrs = attributes(namespace, owner, credential)?;
    if !matches!(
        address.transport(),
        zbus::address::transport::Transport::Unix(_)
    ) {
        return Err(Error::Invalid);
    }
    async_io::block_on(async {
        let deadline = Instant::now() + timeout;
        let conn = bounded(deadline, async {
            zbus::connection::Builder::address(address)
                .map_err(|_| Error::Unavailable)?
                .internal_executor(false)
                .max_queued(16)
                .method_timeout(timeout)
                .build()
                .await
                .map_err(|_| Error::Unavailable)
        })
        .await?;
        // Drive zbus on this owned call's thread, not a detached worker. A
        // single deadline covers connection/authentication and every method.
        let result = bounded(
            deadline,
            futures_lite::future::race(operation(&conn, &attrs, action), async {
                loop {
                    conn.executor().tick().await;
                }
            }),
        )
        .await;
        // Explicitly close the socket, including after cancellation/timeout.
        // Dropping the owned connection also cancels its socket reader task.
        let closed = bounded(Instant::now() + Duration::from_millis(100), async {
            conn.close().await.map_err(|_| Error::Unavailable)
        })
        .await;
        result.and_then(|value| closed.map(|_| value))
    })
}

pub(super) fn read(
    namespace: &str,
    owner: &str,
    credential: &str,
) -> std::result::Result<Zeroizing<String>, String> {
    run(namespace, owner, credential, Operation::Read)
        .map_err(Error::message)?
        .ok_or_else(|| Error::Invalid.message())
}
pub(super) fn create(
    namespace: &str,
    owner: &str,
    credential: &str,
    key: &str,
) -> std::result::Result<(), String> {
    if key.is_empty() || super::validate_key(key).is_err() {
        return Err(Error::Invalid.message());
    }
    run(namespace, owner, credential, Operation::Create(key))
        .map(|_| ())
        .map_err(Error::message)
}
pub(super) fn discard(namespace: &str, owner: &str, credential: &str) {
    let _ = run(namespace, owner, credential, Operation::Discard);
}

#[cfg(all(test, feature = "linux-secret-service-tests"))]
mod native {
    use super::*;
    use std::sync::{Arc, Mutex};
    const OWNER: &str = "11111111-1111-4111-8111-111111111111";
    const CREDENTIAL: &str = "22222222-2222-4222-8222-222222222222";
    const COLLECTION: &str = "/org/freedesktop/secrets/collection/test";
    const ITEM: &str = "/org/freedesktop/secrets/collection/test/item";
    const SESSION: &str = "/org/freedesktop/secrets/session/test";
    fn path(v: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(v).unwrap()
    }
    #[derive(Default)]
    struct Data {
        exists: bool,
        locked: bool,
        duplicate: bool,
        wrong_attributes: bool,
        bad_secret: bool,
        stall: bool,
        creates: usize,
        attrs: Attributes,
        bytes: Vec<u8>,
    }
    type Shared = Arc<Mutex<Data>>;
    struct Service(Shared);
    #[zbus::interface(name = "org.freedesktop.Secret.Service")]
    impl Service {
        async fn search_items(
            &self,
            attrs: Attributes,
        ) -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
            let stall = self.0.lock().unwrap().stall;
            if stall {
                std::future::pending::<()>().await;
            }
            let state = self.0.lock().unwrap();
            if !state.exists || state.attrs != attrs {
                return (vec![], vec![]);
            }
            if state.locked {
                return (vec![], vec![path(ITEM)]);
            }
            if state.duplicate {
                return (vec![path(ITEM), path("/duplicate")], vec![]);
            }
            (vec![path(ITEM)], vec![])
        }
        fn read_alias(&self, alias: &str) -> zbus::fdo::Result<OwnedObjectPath> {
            if alias != "default" {
                return Err(zbus::fdo::Error::InvalidArgs("alias".into()));
            }
            Ok(path(COLLECTION))
        }
        fn open_session(
            &self,
            algorithm: &str,
            input: OwnedValue,
        ) -> zbus::fdo::Result<(OwnedValue, OwnedObjectPath)> {
            if algorithm != "plain" || String::try_from(input).ok().as_deref() != Some("") {
                return Err(zbus::fdo::Error::InvalidArgs("session".into()));
            }
            Ok((
                OwnedValue::from(zbus::zvariant::Str::from("")),
                path(SESSION),
            ))
        }
    }
    struct Collection(Shared);
    #[zbus::interface(name = "org.freedesktop.Secret.Collection")]
    impl Collection {
        #[zbus(property)]
        fn locked(&self) -> bool {
            self.0.lock().unwrap().locked
        }
        fn create_item(
            &self,
            mut properties: HashMap<String, OwnedValue>,
            secret: Secret,
            replace: bool,
        ) -> zbus::fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
            if replace
                || secret.0.as_str() != SESSION
                || !secret.1.is_empty()
                || secret.3 != CONTENT_TYPE
            {
                return Err(zbus::fdo::Error::InvalidArgs("immutable secret".into()));
            }
            let attrs = Attributes::try_from(
                properties
                    .remove("org.freedesktop.Secret.Item.Attributes")
                    .ok_or_else(|| zbus::fdo::Error::InvalidArgs("attributes".into()))?,
            )
            .map_err(|_| zbus::fdo::Error::InvalidArgs("attributes".into()))?;
            let mut state = self.0.lock().unwrap();
            if state.exists || state.locked {
                return Err(zbus::fdo::Error::Failed("existing or locked".into()));
            }
            state.exists = true;
            state.creates += 1;
            state.attrs = attrs;
            state.bytes = secret.2;
            Ok((path(ITEM), path("/")))
        }
    }
    struct Item(Shared);
    #[zbus::interface(name = "org.freedesktop.Secret.Item")]
    impl Item {
        #[zbus(property)]
        fn attributes(&self) -> Attributes {
            let state = self.0.lock().unwrap();
            let mut attrs = state.attrs.clone();
            if state.wrong_attributes {
                attrs.insert("owner".into(), "changed".into());
            }
            attrs
        }
        #[zbus(property)]
        fn locked(&self) -> bool {
            self.0.lock().unwrap().locked
        }
        fn get_secret(&self, session: OwnedObjectPath) -> zbus::fdo::Result<Secret> {
            let state = self.0.lock().unwrap();
            if !state.exists || state.locked || session.as_str() != SESSION {
                return Err(zbus::fdo::Error::Failed("unavailable".into()));
            }
            Ok((
                session,
                vec![],
                if state.bad_secret {
                    vec![0xff]
                } else {
                    state.bytes.clone()
                },
                CONTENT_TYPE.into(),
            ))
        }
        fn delete(&self) -> OwnedObjectPath {
            self.0.lock().unwrap().exists = false;
            path("/")
        }
    }
    struct Fixture {
        connection: Option<Connection>,
        state: Shared,
        address: zbus::Address,
    }
    impl Fixture {
        fn new() -> Self {
            // Run only under an explicitly isolated dbus-run-session. Never
            // read an existing desktop keyring or replace its service owner.
            assert_eq!(
                std::env::var("MEWU_SECRET_SERVICE_TEST_ISOLATED").as_deref(),
                Ok("1")
            );
            let address = zbus::Address::session().unwrap();
            let state = Shared::default();
            let connection = async_io::block_on(async {
                let probe = zbus::connection::Builder::address(address.clone())
                    .unwrap()
                    .build()
                    .await
                    .unwrap();
                let (owned,): (bool,) =
                    call(&probe, BUS, BUS_PATH, BUS, "NameHasOwner", &(SERVICE,))
                        .await
                        .unwrap();
                assert!(
                    !owned,
                    "an existing Secret Service must never be used by these tests"
                );
                probe.close().await.unwrap();
                zbus::connection::Builder::address(address.clone())
                    .unwrap()
                    .name(SERVICE)
                    .unwrap()
                    .serve_at(ROOT, Service(state.clone()))
                    .unwrap()
                    .serve_at(COLLECTION, Collection(state.clone()))
                    .unwrap()
                    .serve_at(ITEM, Item(state.clone()))
                    .unwrap()
                    .build()
                    .await
                    .unwrap()
            });
            Self {
                connection: Some(connection),
                state,
                address,
            }
        }
        fn run(&self, action: Operation<'_>) -> Result<Option<Zeroizing<String>>> {
            run_on(
                self.address.clone(),
                TIMEOUT,
                "connections",
                OWNER,
                CREDENTIAL,
                action,
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            async_io::block_on(self.connection.take().unwrap().close()).unwrap();
        }
    }
    #[test]
    fn native_secret_service_roundtrip_immutable_scope_and_invalid_receipts() {
        let fixture = Fixture::new();
        fixture
            .run(Operation::Create("synthetic-key-only"))
            .unwrap();
        assert_eq!(
            fixture.run(Operation::Read).unwrap().unwrap().as_str(),
            "synthetic-key-only"
        );
        assert_eq!(
            fixture
                .run(Operation::Create("different-synthetic-key"))
                .unwrap_err(),
            Error::Invalid
        );
        assert_eq!(fixture.state.lock().unwrap().creates, 1);
        assert_eq!(
            run_on(
                fixture.address.clone(),
                TIMEOUT,
                "memory",
                OWNER,
                CREDENTIAL,
                Operation::Read
            )
            .unwrap_err(),
            Error::Invalid
        );
        fixture.state.lock().unwrap().locked = true;
        assert_eq!(fixture.run(Operation::Read).unwrap_err(), Error::Locked);
        fixture.state.lock().unwrap().locked = false;
        fixture.state.lock().unwrap().duplicate = true;
        assert_eq!(fixture.run(Operation::Read).unwrap_err(), Error::Invalid);
        fixture.state.lock().unwrap().duplicate = false;
        fixture.state.lock().unwrap().wrong_attributes = true;
        assert_eq!(fixture.run(Operation::Discard).unwrap_err(), Error::Invalid);
        assert!(fixture.state.lock().unwrap().exists);
        fixture.state.lock().unwrap().wrong_attributes = false;
        fixture.state.lock().unwrap().bad_secret = true;
        assert_eq!(fixture.run(Operation::Read).unwrap_err(), Error::Invalid);
        fixture.state.lock().unwrap().bad_secret = false;
        fixture.run(Operation::Discard).unwrap();
        assert_eq!(fixture.run(Operation::Read).unwrap_err(), Error::Invalid);
        fixture.run(Operation::Discard).unwrap();
    }
    #[test]
    fn native_secret_service_stalled_method_obeys_the_total_deadline() {
        let fixture = Fixture::new();
        fixture.state.lock().unwrap().stall = true;
        let start = Instant::now();
        assert_eq!(
            run_on(
                fixture.address.clone(),
                Duration::from_millis(80),
                "connections",
                OWNER,
                CREDENTIAL,
                Operation::Read
            )
            .unwrap_err(),
            Error::Timeout
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn native_secret_service_silent_authentication_closes_its_socket() {
        use std::{io::Read, os::unix::net::UnixListener};
        let directory =
            std::env::temp_dir().join(format!("mewu-secret-bus-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let socket = directory.join("bus");
        let listener = UnixListener::bind(&socket).unwrap();
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = [0u8; 128];
            loop {
                if stream.read(&mut bytes).unwrap() == 0 {
                    break;
                }
            }
        });
        let address =
            zbus::Address::try_from(format!("unix:path={}", socket.display()).as_str()).unwrap();
        let start = Instant::now();
        assert_eq!(
            run_on(
                address,
                Duration::from_millis(80),
                "connections",
                OWNER,
                CREDENTIAL,
                Operation::Read
            )
            .unwrap_err(),
            Error::Timeout
        );
        peer.join().unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        std::fs::remove_file(socket).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
