//! Shared-daemon mode: one server process for every client session with the same configuration.
//!
//! An MCP client launches a stdio server per session, so five agent sessions normally mean five
//! processes, each fetching and parsing the same documents. `serve --daemon` instead makes the
//! launched process a thin **relay**: it copies bytes between its stdio and a Unix socket, and
//! behind that socket a single long-lived **host** process serves every session from one loaded
//! tool set. The relay starts the host itself when none is listening, so client entries only gain
//! a `--daemon` flag.
//!
//! One host per *effective configuration*, not one per machine: the socket name is a hash of the
//! resolved [`Config`] (plus the working directory, which relative spec paths depend on, and the
//! binary, so an upgrade does not talk to an old host). `--group prod` and `--group stg` therefore
//! get separate hosts, while every session asking for the same thing shares one.
//!
//! Exactly-one is enforced by an advisory file lock (`flock`) held for the host's whole life, not
//! by the socket: two relays racing to start a host both spawn one, the loser finds the lock taken
//! and exits 0, and a socket file left behind by a crashed host is safe to delete because whoever
//! holds the lock knows nobody else is serving it.
//!
//! The host exits after `--idle-timeout` seconds with no sessions, which is also how it picks up a
//! changed document: the next session after that starts a fresh host.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ExitCode, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use rmcp::ServiceExt;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::server::OpenApiMcp;
use crate::spec;

/// The hidden `serve` flag that turns a re-executed relay into the host. It is appended to the
/// relay's own argv, so the host resolves exactly the configuration the relay did.
const HOST_FLAG: &str = "--daemon-host";

/// How long a relay waits for a host on top of the document-fetch timeout, which bounds the slow
/// part of host startup.
const STARTUP_GRACE: Duration = Duration::from_secs(15);

/// The files one host owns, all named after the configuration hash.
struct Paths {
    socket: PathBuf,
    lock: PathBuf,
    log: PathBuf,
}

impl Paths {
    /// Derives the paths for `config`, creating the owner-only (0700) runtime directory on first
    /// use. Fails only when that directory cannot be created.
    fn for_config(config: &Config) -> anyhow::Result<Self> {
        // `DefaultHasher::new()` uses fixed keys, so the hash is stable for one build of the
        // binary — which is all that matters, since relay and host are always the same binary.
        let mut hasher = DefaultHasher::new();
        env!("CARGO_PKG_VERSION").hash(&mut hasher);
        std::env::current_exe().ok().hash(&mut hasher);
        std::env::current_dir().ok().hash(&mut hasher);
        // `Config` has no `Hash` (it holds compiled regexes), but its `Debug` form spells out
        // every field, patterns included, which is exactly the identity wanted here.
        format!("{config:?}").hash(&mut hasher);
        let key = format!("{:016x}", hasher.finish());

        let dir = runtime_dir();
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .with_context(|| format!("creating the daemon directory {}", dir.display()))?;

        Ok(Self {
            socket: dir.join(format!("{key}.sock")),
            lock: dir.join(format!("{key}.lock")),
            log: dir.join(format!("{key}.log")),
        })
    }
}

/// `$XDG_RUNTIME_DIR` when set (already per-user and private), else a per-user directory under
/// the system temp dir. Kept short on purpose: a Unix socket path is limited to ~104 bytes.
fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir).join("openapi-as-mcp");
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    std::env::temp_dir().join(format!("openapi-as-mcp-{user}"))
}

/// Runs the relay: connects to (or starts) the host for `config`, then copies stdin to the socket
/// and the socket to stdout until the host closes the session.
///
/// Fails when the host cannot be reached — it crashed on startup (the error carries the tail of
/// its log) or did not start listening in time. On success it exits the process directly rather
/// than returning; see the comment at the end.
pub async fn relay(config: &Config) -> anyhow::Result<ExitCode> {
    let paths = Paths::for_config(config)?;
    let stream = connect_or_start(&paths, config.timeout + STARTUP_GRACE).await?;
    tracing::debug!(socket = %paths.socket.display(), "relaying to the shared daemon");

    let (mut from_host, mut to_host) = stream.into_split();

    // Client → host runs in the background; its end (stdin EOF) is passed on as a half-close so
    // the host sees the session end the same way a stdio server would.
    tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let _ = tokio::io::copy(&mut stdin, &mut to_host).await;
        let _ = to_host.shutdown().await;
    });

    // Host → client decides when the relay is done: it ends when the host closes the session,
    // whether because the client hung up or because the host went away.
    let mut stdout = tokio::io::stdout();
    tokio::io::copy(&mut from_host, &mut stdout)
        .await
        .context("relaying from the daemon")?;
    stdout.flush().await.context("flushing stdout")?;

    // Tokio reads stdin on a blocking thread that cannot be cancelled, and runtime shutdown waits
    // for it — so returning normally could hang until the client closes stdin. Nothing is left
    // to clean up, so leave now.
    std::process::exit(0)
}

/// Connects to the host's socket, starting a host when nothing is listening, and retries with
/// backoff until `timeout`.
///
/// A host that exits 0 lost the lock to another one, which is either still loading (so keep
/// waiting) or shutting down after going idle (so start another once it lets go); both are
/// handled by spawning again on the next attempt. A non-zero exit is a real startup failure.
async fn connect_or_start(paths: &Paths, timeout: Duration) -> anyhow::Result<UnixStream> {
    if let Ok(stream) = UnixStream::connect(&paths.socket).await {
        return Ok(stream);
    }

    let deadline = Instant::now() + timeout;
    let mut delay = Duration::from_millis(25);
    let mut host: Option<Child> = None;

    loop {
        if host.is_none() {
            host = Some(spawn_host(paths)?);
        }

        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_millis(500));

        if let Ok(stream) = UnixStream::connect(&paths.socket).await {
            return Ok(stream);
        }

        let child = host.as_mut().expect("spawned above");
        if let Some(status) = child.try_wait().context("checking on the daemon")? {
            if !status.success() {
                bail!(
                    "the shared daemon failed to start ({status}); from its log at {}:\n{}",
                    paths.log.display(),
                    log_tail(paths)
                );
            }
            host = None;
        }

        if Instant::now() >= deadline {
            bail!(
                "timed out after {timeout:?} waiting for the shared daemon at {}; see its log at {}",
                paths.socket.display(),
                paths.log.display()
            );
        }
    }
}

/// Re-executes this binary with the same arguments plus [`HOST_FLAG`], detached from the relay:
/// its own process group, so a signal aimed at the client's group does not take every session
/// down with it, and stderr appended to the log file, since nobody is reading it.
fn spawn_host(paths: &Paths) -> anyhow::Result<Child> {
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log)
        .with_context(|| format!("opening the daemon log {}", paths.log.display()))?;
    let exe = std::env::current_exe().context("locating this binary")?;

    std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .arg(HOST_FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .process_group(0)
        .spawn()
        .context("starting the shared daemon")
}

/// The last lines of the host's log, for a startup error; empty when it cannot be read.
fn log_tail(paths: &Paths) -> String {
    let text = fs::read_to_string(&paths.log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(20)..].join("\n")
}

/// Runs the host: takes the lock, loads the documents once, and serves one MCP session per socket
/// connection until nothing has been connected for `idle` (zero means never).
///
/// Returns success without serving when another host already holds the lock. Fails when the
/// documents do not load or the socket cannot be bound — the relay reports that from the log.
pub async fn host(config: Config, idle: Duration) -> anyhow::Result<ExitCode> {
    let paths = Paths::for_config(&config)?;

    // The lock lives exactly as long as this `File`, i.e. the whole function; the OS drops it if
    // the process dies, so a crash never leaves a host "running".
    let lock = File::create(&paths.lock)
        .with_context(|| format!("opening the lock file {}", paths.lock.display()))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            tracing::info!("another daemon already serves this configuration");
            return Ok(ExitCode::SUCCESS);
        }
        Err(TryLockError::Error(err)) => return Err(err).context("locking the daemon lock file"),
    }

    let apis = spec::load_all(&config.specs(), config.timeout).await?;
    let server = Arc::new(OpenApiMcp::new(&config, apis)?);

    // Holding the lock means no live host owns the socket file, so an existing one is stale.
    let _ = fs::remove_file(&paths.socket);
    let listener = UnixListener::bind(&paths.socket)
        .with_context(|| format!("binding {}", paths.socket.display()))?;
    fs::set_permissions(&paths.socket, fs::Permissions::from_mode(0o600))
        .context("restricting the socket to its owner")?;

    tracing::info!(
        socket = %paths.socket.display(),
        tools = server.tools().len(),
        idle_timeout = ?idle,
        "daemon listening"
    );

    // Each session reports its end on this channel, so the accept loop can count live sessions
    // without sharing a counter across tasks.
    let (ended_tx, mut ended_rx) = mpsc::unbounded_channel::<()>();
    let mut sessions = 0usize;

    loop {
        // Rebuilt on every iteration, so the idle clock restarts whenever something happens and
        // only runs while no session is open.
        let idle_expired = async {
            if sessions == 0 && !idle.is_zero() {
                tokio::time::sleep(idle).await;
            } else {
                std::future::pending::<()>().await;
            }
        };

        tokio::select! {
            accepted = listener.accept() => {
                let stream = match accepted {
                    Ok((stream, _)) => stream,
                    Err(err) => {
                        tracing::warn!(error = %err, "accepting a session failed");
                        continue;
                    }
                };
                sessions += 1;
                tracing::info!(sessions, "session opened");

                let server = Arc::clone(&server);
                let ended = ended_tx.clone();
                tokio::spawn(async move {
                    match server.serve(stream.into_split()).await {
                        Ok(running) => {
                            let _ = running.waiting().await;
                        }
                        Err(err) => tracing::warn!(error = %err, "session failed to initialize"),
                    }
                    let _ = ended.send(());
                });
            }
            Some(()) = ended_rx.recv() => {
                sessions -= 1;
                tracing::info!(sessions, "session closed");
            }
            () = idle_expired => {
                tracing::info!("idle, shutting down");
                break;
            }
        }
    }

    // Remove the socket while still holding the lock, so the next host never deletes a live one.
    let _ = fs::remove_file(&paths.socket);
    drop(lock);
    Ok(ExitCode::SUCCESS)
}
