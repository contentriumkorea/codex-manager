use serde_json::Value;
use std::collections::HashSet;
use std::io::{BufRead, Write};

pub struct TimedReader {
    receiver: std::sync::mpsc::Receiver<Result<Value, String>>,
}
impl TimedReader {
    pub fn new<R: BufRead + Send + 'static>(mut reader: R) -> Self {
        let (sender, receiver) = std::sync::mpsc::sync_channel(64);
        std::thread::spawn(move || {
            let mut line = String::new();
            loop {
                line.clear();
                let result = match reader.read_line(&mut line) {
                    Ok(0) => Err("app-server closed before responding".to_owned()),
                    Ok(_) => serde_json::from_str::<Value>(&line)
                        .map_err(|e| format!("invalid app-server message: {e}")),
                    Err(e) => Err(e.to_string()),
                };
                let terminal = result.is_err();
                // Notifications need no acknowledgement. Do not fill the bounded response
                // queue while a client is idle or writing its next request.
                if result.as_ref().is_ok_and(|v| v.get("id").is_none()) {
                    continue;
                }
                if sender.send(result).is_err() || terminal {
                    break;
                }
            }
        });
        Self { receiver }
    }
    pub fn response(&mut self, id: u64, timeout: std::time::Duration) -> Result<Value, String> {
        let deadline = std::time::Instant::now()
            .checked_add(timeout)
            .ok_or("timeout overflow")?;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let message = self
                .receiver
                .recv_timeout(remaining)
                .map_err(|e| match e {
                    std::sync::mpsc::RecvTimeoutError::Timeout => "app-server timeout",
                    std::sync::mpsc::RecvTimeoutError::Disconnected => "app-server disconnected",
                })??;
            if message.get("method").is_some() && message.get("id").is_some() {
                return Err("unsupported server request".into());
            }
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(format!(
                    "app-server error: {}",
                    error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                ));
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| "response has no result".into());
        }
    }
}

pub struct CodexProcess {
    child: std::process::Child,
    client: RpcClient<TimedWriter>,
}
impl CodexProcess {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    pub fn start(
        binary: &std::path::Path,
        home: &std::path::Path,
        sqlite_home: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        use std::process::{Command, Stdio};
        if !binary.is_absolute() || !home.is_absolute() || !home.is_dir() {
            return Err("an absolute executable and existing home are required".into());
        }
        let mut command = Command::new(binary);
        if let Some(sqlite) = sqlite_home {
            if !sqlite.is_absolute() || !sqlite.is_dir() {
                return Err("invalid SQLite home".into());
            }
            let config = format!(
                "sqlite_home={}",
                serde_json::to_string(sqlite).map_err(|e| e.to_string())?
            );
            command
                .args(["-c", &config])
                .env("CODEX_SQLITE_HOME", sqlite);
        } else {
            command.env_remove("CODEX_SQLITE_HOME");
        }
        command
            .args(["app-server", "--listen", "stdio://"])
            .env("CODEX_HOME", home)
            .env("RUST_LOG", "error")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("could not start Codex: {e}"))?;
        let streams = (child.stdout.take(), child.stdin.take());
        let (stdout, stdin) = match streams {
            (Some(stdout), Some(stdin)) => (stdout, stdin),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("missing server pipes".into());
            }
        };
        let mut server = Self {
            child,
            client: RpcClient::new(std::io::BufReader::new(stdout), TimedWriter::new(stdin)),
        };
        server.client.initialize()?;
        Ok(server)
    }
    pub fn projects(&mut self) -> Result<Vec<Value>, String> {
        self.client.list_projects()
    }
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.client.call(method, params)
    }
    pub fn threads(
        &mut self,
        filter: ProjectFilter<'_>,
        archived: bool,
    ) -> Result<Vec<Value>, String> {
        self.client.list_threads(filter, archived)
    }
}

type WriteRequest = (Vec<u8>, std::sync::mpsc::SyncSender<Result<(), String>>);
struct TimedWriter {
    sender: std::sync::mpsc::SyncSender<WriteRequest>,
}
impl TimedWriter {
    fn new(mut pipe: std::process::ChildStdin) -> Self {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<(
            Vec<u8>,
            std::sync::mpsc::SyncSender<Result<(), String>>,
        )>(1);
        std::thread::spawn(move || {
            while let Ok((bytes, done)) = receiver.recv() {
                let result = pipe
                    .write_all(&bytes)
                    .and_then(|_| pipe.flush())
                    .map_err(|e| e.to_string());
                let failed = result.is_err();
                let _ = done.send(result);
                if failed {
                    break;
                }
            }
        });
        Self { sender }
    }
}
impl Write for TimedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send((bytes.to_vec(), sender))
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "app-server writer unavailable",
                )
            })?;
        receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "app-server write timeout")
            })?
            .map_err(std::io::Error::other)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Drop for CodexProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct RpcClient<W> {
    reader: TimedReader,
    writer: W,
    next_id: u64,
    initialized: bool,
}

impl<W: Write> RpcClient<W> {
    pub fn new<R: BufRead + Send + 'static>(reader: R, writer: W) -> Self {
        Self {
            reader: TimedReader::new(reader),
            writer,
            next_id: 0,
            initialized: false,
        }
    }

    fn send(&mut self, message: Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&message).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        self.writer
            .write_all(&bytes)
            .and_then(|_| self.writer.flush())
            .map_err(|e| e.to_string())
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id = self.next_id.checked_add(1).ok_or("request id exhausted")?;
        let id = self.next_id;
        self.send(serde_json::json!({"id":id,"method":method,"params":params}))?;
        self.reader.response(id, std::time::Duration::from_secs(30))
    }

    pub fn initialize(&mut self) -> Result<(), String> {
        if self.initialized {
            return Err("already initialized".into());
        }
        self.call(
            "initialize",
            serde_json::json!({
                "clientInfo":{"name":"codex_manager_rust","version":"0.1.0"},
                "capabilities":{"experimentalApi":true}
            }),
        )?;
        self.send(serde_json::json!({"method":"initialized"}))?;
        self.initialized = true;
        Ok(())
    }

    fn pages(&mut self, method: &str, base: Value) -> Result<Vec<Value>, String> {
        if !self.initialized {
            return Err("not initialized".into());
        }
        collect_pages(|cursor| {
            let mut params = base.clone();
            if let Some(cursor) = cursor {
                params["cursor"] = Value::String(cursor.to_owned());
            }
            self.call(method, params)
        })
    }

    pub fn list_projects(&mut self) -> Result<Vec<Value>, String> {
        self.pages("project/list", serde_json::json!({}))
    }
    pub fn list_threads(
        &mut self,
        filter: ProjectFilter<'_>,
        archived: bool,
    ) -> Result<Vec<Value>, String> {
        self.pages("thread/list", thread_list_params(filter, archived))
    }
}

pub enum ProjectFilter<'a> {
    All,
    Unassigned,
    Project(&'a str),
}

pub fn thread_list_params(filter: ProjectFilter<'_>, archived: bool) -> Value {
    let mut params = serde_json::json!({"archived": archived, "useStateDbOnly": true});
    match filter {
        ProjectFilter::All => {}
        ProjectFilter::Unassigned => params["projectId"] = Value::Null,
        ProjectFilter::Project(id) => params["projectId"] = Value::String(id.to_owned()),
    }
    params
}

pub fn read_response<R: BufRead>(reader: &mut R, id: u64) -> Result<Value, String> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err("app-server closed before responding".into());
        }
        let message: Value =
            serde_json::from_str(&line).map_err(|e| format!("invalid app-server message: {e}"))?;
        if message.get("method").is_some() && message.get("id").is_some() {
            return Err("unsupported server request".into());
        }
        if message.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = message.get("error") {
            return Err(format!(
                "app-server error: {}",
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ));
        }
        return message
            .get("result")
            .cloned()
            .ok_or_else(|| "response has no result".into());
    }
}

pub fn collect_pages<F>(mut fetch: F) -> Result<Vec<Value>, String>
where
    F: FnMut(Option<&str>) -> Result<Value, String>,
{
    let mut cursor: Option<String> = None;
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    loop {
        let page = fetch(cursor.as_deref())?;
        let data = page
            .get("data")
            .and_then(Value::as_array)
            .ok_or("invalid page data")?;
        result.extend(data.iter().cloned());
        match page.get("nextCursor") {
            None | Some(Value::Null) => return Ok(result),
            Some(Value::String(next)) if !next.is_empty() => {
                if !seen.insert(next.clone()) {
                    return Err("repeated cursor".into());
                }
                cursor = Some(next.clone());
            }
            _ => return Err("invalid cursor".into()),
        }
    }
}
