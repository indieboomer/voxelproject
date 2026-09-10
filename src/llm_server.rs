use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Bundled llama.cpp server binary and the coding model it's expected to
/// serve, relative to `llm-runtime/server` (see `find_server_dir`).
const SERVER_EXE: &str = "llama-server.exe";
const SERVER_SUBDIR: &str = "llm-runtime/server";
const MODEL_RELATIVE: &str = "../models/qwen2.5-coder-7b-instruct-q4_k_m.gguf";

const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_millis(500);
// One host submits one pipeline at a time. Auto-sizing reserves a very large
// context and several slots on the bundled server, competing with game rendering.
const INFERENCE_ARGS: &[&str] = &["--ctx-size", "32768", "--parallel", "1"];

/// Makes sure a local `llama-server` is answering at `llm_url`, launching
/// the bundled one if it isn't. Meant to be called on a background thread
/// at startup so a forgotten-after-reboot server doesn't leave rule
/// generation silently broken until the player notices and starts it by
/// hand.
///
/// No-ops for a non-loopback URL -- a server we didn't launch isn't ours to
/// manage -- or if the bundled server files can't be found near the game
/// executable.
pub fn ensure_running(llm_url: &str) {
    let Some((host, port)) = parse_host_port(llm_url) else {
        log::warn!("llm_server: couldn't parse host/port out of '{llm_url}', skipping auto-start");
        return;
    };
    if !is_loopback(&host) {
        return;
    }
    if is_reachable(llm_url) {
        return;
    }

    let Some(server_dir) = find_server_dir() else {
        log::warn!(
            "llama-server not reachable at {llm_url} and '{SERVER_SUBDIR}' wasn't found near \
             the game executable -- start it manually"
        );
        return;
    };

    log::info!("llama-server not reachable at {llm_url}, starting the bundled one...");
    if let Err(e) = spawn_server(&server_dir, &host, port) {
        log::warn!("failed to auto-start llama-server: {e}");
    }
}

fn is_reachable(llm_url: &str) -> bool {
    let url = format!("{}/health", llm_url.trim_end_matches('/'));
    ureq::get(&url).timeout(HEALTH_CHECK_TIMEOUT).call().is_ok()
}

/// Extracts `(host, port)` from a `http://host:port` style URL without
/// pulling in a full URL-parsing dependency for this one call site.
fn parse_host_port(url: &str) -> Option<(String, u16)> {
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = without_scheme
        .split(['/', '?'])
        .next()
        .unwrap_or(without_scheme);
    let (host, port) = authority.rsplit_once(':')?;
    Some((host.to_string(), port.parse().ok()?))
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// Looks for `llm-runtime/server/llama-server.exe` near the current working
/// directory and near the running executable, so auto-start still works
/// whether the game was launched via `cargo run` (cwd = project root) or by
/// double-clicking `target/{debug,release}/voxelproject.exe`.
fn find_server_dir() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        // Walk up a few levels to cover target/debug and target/release.
        for _ in 0..4 {
            if let Some(d) = dir {
                candidates.push(d.clone());
                dir = d.parent().map(Path::to_path_buf);
            } else {
                break;
            }
        }
    }

    candidates.into_iter().find_map(|root| {
        let server_dir = root.join(SERVER_SUBDIR);
        if server_dir.join(SERVER_EXE).is_file() {
            Some(server_dir)
        } else {
            None
        }
    })
}

fn spawn_server(server_dir: &Path, host: &str, port: u16) -> Result<(), String> {
    let log_path = server_dir.join("server.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("couldn't open {}: {e}", log_path.display()))?;
    let log_file_err = log_file
        .try_clone()
        .map_err(|e| format!("couldn't duplicate log handle: {e}"))?;

    // Command::new resolves a relative program name against *our* cwd and
    // PATH, not the child's `current_dir` -- has to be the joined absolute
    // path or this silently fails to find the exe when launched from
    // anywhere other than server_dir itself.
    let mut cmd = Command::new(server_dir.join(SERVER_EXE));
    cmd.current_dir(server_dir)
        .arg("--model")
        .arg(MODEL_RELATIVE)
        .arg("--host")
        .arg(host)
        .arg("--port")
        .arg(port.to_string())
        .args(INFERENCE_ARGS)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_file_err));

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console popup. Deliberately *not*
        // DETACHED_PROCESS/CREATE_NEW_PROCESS_GROUP -- the server should
        // keep running after the game exits, which is already the default
        // (Windows doesn't kill child processes when the parent exits), and
        // this stays as a plain child so it dies with the game during
        // development if that ever turns out to be preferable.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.spawn()
        .map(|_child| ())
        .map_err(|e| format!("failed to spawn {SERVER_EXE}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_host_port_reads_default_llm_url() {
        assert_eq!(
            parse_host_port(crate::net::DEFAULT_LLM_URL),
            Some(("127.0.0.1".to_string(), 8090))
        );
    }

    #[test]
    fn parse_host_port_ignores_trailing_path() {
        assert_eq!(
            parse_host_port("http://localhost:8090/v1/chat/completions"),
            Some(("localhost".to_string(), 8090))
        );
    }

    #[test]
    fn parse_host_port_rejects_missing_port() {
        assert_eq!(parse_host_port("http://localhost"), None);
    }

    #[test]
    fn loopback_hosts_are_recognized() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("localhost"));
        assert!(!is_loopback("192.168.1.50"));
        assert!(!is_loopback("example.com"));
    }
}
