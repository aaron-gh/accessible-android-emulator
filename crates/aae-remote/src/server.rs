//! The server: connections from phones, signing in, and their commands.
//!
//! Everything goes over one WebSocket in TLS. Text messages are JSON objects
//! with a "type". The phone sends:
//!
//! - `{"type":"pair","name":…,"nonce":…,"proof":…}` to pair with a code, or
//!   `{"type":"hello","client":…,"token":…}` once paired. Nothing else is
//!   accepted first.
//! - `{"type":"call","id":1,"method":"devices.list","params":{…}}`, answered
//!   with `{"type":"result","id":1,"result":…}` or
//!   `{"type":"error","id":1,"message":…}`. Long calls send
//!   `{"type":"progress","id":1,"message":…,"percent":…}` on the way.
//! - `{"type":"key","code":30,"down":true}`, with Linux key codes, and
//!   `{"type":"touch","points":[{"id":0,"x":…,"y":…,"down":true}]}`, in the
//!   device's screen pixels, for the attached device. These aren't answered,
//!   so they go as fast as they come.
//!
//! The server also sends `{"type":"event",…}` messages, such as the attached
//! device's vibration, and binary messages: the attached device's sound, a 1
//! then 16-bit little-endian stereo samples at 48 kHz.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use aae_ffi::{AaeError, DeviceProfile, Engine, LicenceInfo, ScreenReaderSource, Session};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::json;
use crate::media;
use crate::security::{self, Clients, Identity, Pairing};

/// The port AAE listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 47735;
/// The sound sent to phones: 48 kHz stereo.
pub const AUDIO_RATE: u32 = 48_000;
/// The first byte of a binary message carrying sound.
pub const AUDIO_FRAME: u8 = 1;
/// The first byte of a binary message from the phone carrying part of a
/// file it's sending, after `upload.begin`: then the upload's number, four
/// bytes big-endian, then the data.
pub const UPLOAD_FRAME: u8 = 2;

pub struct Server {
    pub engine: Arc<Engine>,
    pub identity: Identity,
    pub fingerprint: String,
    pub clients: Clients,
    pub pairing: Pairing,
    /// This computer's name, as phones list it.
    pub name: String,
    sessions: tokio::sync::Mutex<HashMap<String, Arc<Session>>>,
    /// Things worth telling whoever runs the server, such as a phone pairing.
    notices: Mutex<Option<mpsc::UnboundedSender<String>>>,
}

impl Server {
    pub fn new(engine: Arc<Engine>, name: String) -> std::io::Result<Arc<Server>> {
        let identity = Identity::load_or_create()?;
        Ok(Arc::new(Server {
            engine,
            fingerprint: identity.fingerprint(),
            identity,
            clients: Clients::load(),
            pairing: Pairing::default(),
            name,
            sessions: tokio::sync::Mutex::new(HashMap::new()),
            notices: Mutex::new(None),
        }))
    }

    /// Where notices go, such as "Pixel paired."
    pub fn on_notice(&self, sender: mpsc::UnboundedSender<String>) {
        *self.notices.lock().unwrap() = Some(sender);
    }

    fn notice(&self, text: String) {
        tracing::info!("{text}");
        if let Some(sender) = self.notices.lock().unwrap().as_ref() {
            let _ = sender.send(text);
        }
    }

    /// The connection to a running device, made the first time it's needed.
    pub async fn session(&self, id: &str) -> Result<Arc<Session>, AaeError> {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get(id) {
            return Ok(session.clone());
        }
        let session = self.engine.open_session(id.to_string()).await?;
        sessions.insert(id.to_string(), session.clone());
        Ok(session)
    }

    async fn forget_session(&self, id: &str) {
        self.sessions.lock().await.remove(id);
    }

    /// Accepts phones on `port`, on every network address, until the task
    /// is dropped.
    pub async fn run(self: Arc<Self>, port: u16) -> std::io::Result<()> {
        let tls = tokio_rustls::TlsAcceptor::from(Arc::new(self.tls_config()?));
        let listener = TcpListener::bind(("0.0.0.0", port)).await?;
        loop {
            let (socket, address) = listener.accept().await?;
            let server = self.clone();
            let tls = tls.clone();
            tokio::spawn(async move {
                let _ = socket.set_nodelay(true);
                match tls.accept(socket).await {
                    Ok(stream) => match tokio_tungstenite::accept_async(stream).await {
                        Ok(ws) => server.connection(ws, address).await,
                        Err(e) => tracing::debug!("{address}: not a WebSocket: {e}"),
                    },
                    Err(e) => tracing::debug!("{address}: TLS failed: {e}"),
                }
            });
        }
    }

    fn tls_config(&self) -> std::io::Result<rustls::ServerConfig> {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
        rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(std::io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(self.identity.certificate.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.identity.key.clone())),
        )
        .map_err(std::io::Error::other)
    }

    async fn connection<S>(
        self: Arc<Self>,
        ws: tokio_tungstenite::WebSocketStream<S>,
        address: SocketAddr,
    ) where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (mut sink, mut stream) = ws.split();
        let (out, mut outgoing) = mpsc::unbounded_channel::<Message>();
        let writer = tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                if sink.send(message).await.is_err() {
                    break;
                }
            }
        });
        let mut phone = Phone {
            server: self.clone(),
            out: Out(out),
            client: None,
            attached: None,
            uploads: HashMap::new(),
            next_upload: 1,
        };
        while let Some(Ok(message)) = stream.next().await {
            let text = match message {
                Message::Text(text) => text,
                Message::Binary(bytes) => {
                    phone.upload_part(&bytes);
                    continue;
                }
                _ => continue,
            };
            let Ok(message) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if !phone.handle(message).await {
                break;
            }
        }
        if let Some(client) = &phone.client {
            tracing::info!("{} at {address} disconnected", client.name);
        }
        phone.detach();
        for (_, (path, _)) in phone.uploads.drain() {
            let _ = std::fs::remove_file(path);
        }
        writer.abort();
    }
}

/// Sends messages to one phone.
#[derive(Clone)]
pub struct Out(mpsc::UnboundedSender<Message>);

impl Out {
    pub fn json(&self, value: Value) {
        let _ = self.0.send(Message::text(value.to_string()));
    }

    pub fn binary(&self, bytes: Vec<u8>) -> bool {
        self.0.send(Message::binary(bytes)).is_ok()
    }

    fn result(&self, id: &Value, result: Result<Value, String>) {
        self.json(match result {
            Ok(result) => json!({"type": "result", "id": id, "result": result}),
            Err(message) => json!({"type": "error", "id": id, "message": message}),
        });
    }
}

/// One connected phone.
struct Phone {
    server: Arc<Server>,
    out: Out,
    client: Option<security::Client>,
    /// The device whose sound and vibration go to this phone.
    attached: Option<media::Attachment>,
    /// Files being sent from the phone, such as APKs, by number.
    uploads: HashMap<u32, (std::path::PathBuf, std::fs::File)>,
    next_upload: u32,
}

impl Phone {
    /// Handles a message. False ends the connection.
    async fn handle(&mut self, message: Value) -> bool {
        let kind = message["type"].as_str().unwrap_or("");
        let text = |key: &str| message[key].as_str().unwrap_or("").to_string();
        if self.client.is_none() {
            return match kind {
                "pair" => self.pair(&text("name"), &text("nonce"), &text("proof")),
                "hello" => self.hello(&text("client"), &text("token")),
                _ => false,
            };
        }
        match kind {
            "key" => {
                if let (Some(attached), Some(code)) = (&self.attached, message["code"].as_u64()) {
                    attached
                        .session
                        .evdev_key(code as u16, message["down"].as_bool().unwrap_or(false));
                }
            }
            "touch" => {
                if let Some(attached) = &self.attached {
                    attached.touch(&message["points"]);
                }
            }
            "call" => {
                let id = message["id"].clone();
                let method = text("method");
                let params = message["params"].clone();
                match method.as_str() {
                    // These change the connection, so they're handled here.
                    "device.attach" => {
                        let result = self.attach(params["id"].as_str().unwrap_or("")).await;
                        self.out.result(&id, result);
                    }
                    "device.detach" => {
                        self.detach();
                        self.out.result(&id, Ok(Value::Null));
                    }
                    "upload.begin" => {
                        let result = self.begin_upload(params["name"].as_str().unwrap_or("upload"));
                        self.out.result(&id, result);
                    }
                    "tools.install" => {
                        // Installs a file the phone sent, then deletes it.
                        let upload = params["upload"].as_u64().unwrap_or(0) as u32;
                        let Some((path, _)) = self.uploads.remove(&upload) else {
                            self.out.result(&id, Err("That file wasn't sent.".into()));
                            return true;
                        };
                        let server = self.server.clone();
                        let out = self.out.clone();
                        let device = params["id"].as_str().unwrap_or("").to_string();
                        tokio::spawn(async move {
                            let result = async {
                                let session = server.session(&device).await?;
                                let installed = session.install_apk(path.to_string_lossy().into_owned()).await?;
                                Ok::<Value, AaeError>(json!({
                                    "package": installed.package,
                                    "parts": installed.parts.iter().map(crate::tools::part).collect::<Vec<_>>(),
                                }))
                            }
                            .await;
                            let _ = std::fs::remove_file(&path);
                            out.result(&id, result.map_err(|e| e.to_string()));
                        });
                    }
                    _ => {
                        let server = self.server.clone();
                        let out = self.out.clone();
                        tokio::spawn(async move {
                            let progress = Progress {
                                out: out.clone(),
                                id: id.clone(),
                            };
                            let result = call(&server, &method, &params, progress)
                                .await
                                .map_err(|e| e.to_string());
                            out.result(&id, result);
                        });
                    }
                }
            }
            _ => {}
        }
        true
    }

    /// Starts receiving a file, kept in a temporary folder until used.
    fn begin_upload(&mut self, name: &str) -> Result<Value, String> {
        let number = self.next_upload;
        self.next_upload += 1;
        let dir = std::env::temp_dir().join("aae-uploads");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // Only the file's own name, so it can't point anywhere else.
        let safe: String = name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("upload")
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || ".-_".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let path = dir.join(format!("{}-{number}-{safe}", security::random_text(8)));
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        self.uploads.insert(number, (path, file));
        Ok(json!(number))
    }

    fn upload_part(&mut self, bytes: &[u8]) {
        use std::io::Write;
        if bytes.len() < 5 || bytes[0] != UPLOAD_FRAME {
            return;
        }
        let number = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
        if let Some((_, file)) = self.uploads.get_mut(&number) {
            let _ = file.write_all(&bytes[5..]);
        }
    }

    fn pair(&mut self, name: &str, nonce: &str, proof: &str) -> bool {
        let server = &self.server;
        match server.pairing.check(&server.fingerprint, nonce, proof) {
            Ok(code) => {
                let name = if name.trim().is_empty() {
                    "A phone"
                } else {
                    name.trim()
                };
                let (client, token) = server.clients.add(name);
                self.out.json(json!({
                    "type": "paired",
                    "client": client.id,
                    "token": token,
                    "server": server.name,
                    "proof": security::server_proof(&code, &server.fingerprint, nonce),
                }));
                server.notice(format!("{} paired.", client.name));
                self.client = Some(client);
                true
            }
            Err(why) => {
                self.out.json(json!({"type": "denied", "message": why}));
                false
            }
        }
    }

    fn hello(&mut self, id: &str, token: &str) -> bool {
        match self.server.clients.find(id, token) {
            Some(client) => {
                self.out.json(json!({
                    "type": "welcome",
                    "server": self.server.name,
                    "version": env!("CARGO_PKG_VERSION"),
                }));
                tracing::info!("{} connected", client.name);
                self.client = Some(client);
                true
            }
            None => {
                self.out.json(json!({
                    "type": "denied",
                    "message": "This phone isn't paired with this computer any more. Pair it again.",
                }));
                false
            }
        }
    }

    /// Sends a device's sound and vibration to this phone, and its keys and
    /// touches to the device, until another is attached or the phone goes.
    async fn attach(&mut self, id: &str) -> Result<Value, String> {
        self.detach();
        let session = self.server.session(id).await.map_err(|e| e.to_string())?;
        let size = session
            .clone()
            .screen_size()
            .await
            .map_err(|e| e.to_string())?;
        self.attached = Some(media::Attachment::start(session, self.out.clone()).await);
        Ok(json!({"width": size.x, "height": size.y}))
    }

    fn detach(&mut self) {
        if let Some(attached) = self.attached.take() {
            attached.stop();
        }
    }
}

/// Sends a long call's progress to the phone.
#[derive(Clone)]
struct Progress {
    out: Out,
    id: Value,
}

impl aae_ffi::ProgressListener for Progress {
    fn progress(&self, message: String) {
        self.out
            .json(json!({"type": "progress", "id": self.id, "message": message}));
    }
}

impl aae_ffi::DownloadListener for Progress {
    fn downloaded(&self, percent: u32) {
        self.out
            .json(json!({"type": "progress", "id": self.id, "percent": percent}));
    }

    fn stage(&self, message: String) {
        self.out
            .json(json!({"type": "progress", "id": self.id, "message": message}));
    }
}

fn failed(message: impl Into<String>) -> AaeError {
    AaeError::Failed {
        message: message.into(),
    }
}

fn param(params: &Value, key: &str) -> Result<String, AaeError> {
    params[key]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| failed(format!("The request needs \"{key}\".")))
}

/// Runs a command from the phone.
async fn call(
    server: &Arc<Server>,
    method: &str,
    params: &Value,
    progress: Progress,
) -> Result<Value, AaeError> {
    if let Some(result) = crate::tools::call(server, method, params).await {
        return result;
    }
    let engine = &server.engine;
    let id = || param(params, "id");
    Ok(match method {
        "server.info" => json!({
            "name": server.name,
            "version": env!("CARGO_PKG_VERSION"),
            "needs_setup": engine.needs_setup(),
        }),

        // Devices.
        "devices.list" => json!(
            engine
                .devices()?
                .iter()
                .map(json::device)
                .collect::<Vec<_>>()
        ),
        "devices.create" => {
            let profile = match params["profile"].as_str().unwrap_or("phone") {
                "small-phone" => DeviceProfile::SmallPhone,
                "tablet" => DeviceProfile::Tablet,
                _ => DeviceProfile::Phone,
            };
            json::device(&engine.create_device(
                param(params, "name")?,
                param(params, "sysdir")?,
                profile,
            )?)
        }
        "device.start" => {
            let id = id()?;
            engine
                .start_device(
                    id.clone(),
                    engine.default_screen_reader(),
                    params["volume_boost"].as_bool().unwrap_or(true),
                    Arc::new(progress),
                )
                .await?;
            let device = engine.devices()?.into_iter().find(|d| d.id == id);
            json!({
                // Started without a screen reader, which the phone asks about.
                "ask_screen_reader": device
                    .is_some_and(|d| d.screen_reader.is_none() && !d.screen_reader_declined),
            })
        }
        "device.stop" => {
            let id = id()?;
            engine.stop_device(id.clone()).await?;
            server.forget_session(&id).await;
            Value::Null
        }
        "device.restart" => {
            engine.restart_device(id()?, Arc::new(progress)).await?;
            Value::Null
        }
        "device.wipe" => {
            let id = id()?;
            server.forget_session(&id).await;
            engine.wipe_device(id, Arc::new(progress)).await?;
            Value::Null
        }
        "device.delete" => json!(engine.delete_device(id()?)?),
        "device.rename" => json::device(&engine.rename_device(id()?, param(params, "name")?)?),
        "device.copy" => json::device(&engine.clone_device(id()?, param(params, "name")?)?),
        "device.size" => json!(engine.device_size(id()?)?),
        "device.screen_reader" => match params["choice"].as_str() {
            Some("backtalk") => json!(
                engine
                    .add_screen_reader(id()?, ScreenReaderSource::Backtalk)
                    .await?
            ),
            _ => {
                engine.decline_screen_reader(id()?)?;
                Value::Null
            }
        },
        "device.press" => {
            server
                .session(&id()?)
                .await?
                .press(param(params, "key")?)
                .await?;
            Value::Null
        }
        "device.type" => {
            server
                .session(&id()?)
                .await?
                .type_text(param(params, "text")?)
                .await?;
            Value::Null
        }
        "device.notifications" => {
            server
                .session(&id()?)
                .await?
                .shell("cmd statusbar expand-notifications".into())
                .await?;
            Value::Null
        }
        "device.quick_settings" => {
            server
                .session(&id()?)
                .await?
                .shell("cmd statusbar expand-settings".into())
                .await?;
            Value::Null
        }
        "device.gesture" => {
            server
                .session(&id()?)
                .await?
                .perform_gesture(param(params, "gesture")?, None)
                .await?;
            Value::Null
        }
        "device.status" => {
            // As the desktop apps say it: the device, its state, then its screen reader.
            let id = id()?;
            let device = engine
                .devices()?
                .into_iter()
                .find(|d| d.id == id)
                .ok_or_else(|| failed("That device no longer exists."))?;
            let mut parts = vec![
                format!("{}, {}.", device.name, device.android),
                if device.running {
                    "Running."
                } else {
                    "Stopped."
                }
                .to_string(),
            ];
            if device.running {
                parts.push(server.session(&id).await?.screen_reader_status().await?);
            }
            json!(parts.join(" "))
        }
        "device.rotate" => json!(
            server
                .session(&id()?)
                .await?
                .rotate(params["left"].as_bool().unwrap_or(true))
                .await?
        ),
        "device.volume" => {
            let volume = params["volume"].as_f64().unwrap_or(1.0) as f32;
            server.session(&id()?).await?.set_audio_volume(volume)?;
            Value::Null
        }

        // Android versions.
        "images.list" => json!(engine.images().iter().map(json::image).collect::<Vec<_>>()),
        "images.installed" => json!(
            engine
                .installed_images()
                .await?
                .iter()
                .map(json::installed_image)
                .collect::<Vec<_>>()
        ),
        "images.remove" => json!(engine.remove_image(param(params, "sysdir")?).await?),
        "versions.list" => json!(
            engine
                .versions(
                    params["refresh"].as_bool().unwrap_or(false),
                    params["previews"].as_bool().unwrap_or(false),
                )
                .await?
                .iter()
                .map(json::version)
                .collect::<Vec<_>>()
        ),
        "versions.licence" => match engine.licence_to_accept(id()?).await? {
            Some(licence) => json::licence(&licence),
            None => Value::Null,
        },
        "versions.install" => {
            json::image(&engine.install_version(id()?, Arc::new(progress)).await?)
        }

        // Google's licences, accepted by the person on the phone.
        "licence.accept" => {
            engine.accept_licence(LicenceInfo {
                id: param(params, "licence")?,
                text: param(params, "text")?,
            })?;
            Value::Null
        }

        // The emulator and tools.
        "setup.status" => json::setup(
            &engine
                .setup_status(params["refresh"].as_bool().unwrap_or(false))
                .await?,
        ),
        "setup.licence" => match engine
            .tools_licence(params["update"].as_bool().unwrap_or(false))
            .await?
        {
            Some(licence) => json::licence(&licence),
            None => Value::Null,
        },
        "setup.install" => {
            engine
                .install_tools(
                    params["update"].as_bool().unwrap_or(false),
                    Arc::new(progress),
                )
                .await?;
            Value::Null
        }

        _ => {
            return Err(failed(format!(
                "This computer's AAE doesn't know \"{method}\". Update AAE on it."
            )));
        }
    })
}
