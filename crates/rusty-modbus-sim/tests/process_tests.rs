//! Process-level tests for the simulator executable.

use std::fs;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rusty_modbus_client::{ClientConfig, ModbusClient};
use rusty_modbus_types::UnitId;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader, Lines};
use tokio::process::{ChildStdout, Command};
use tokio::time;

const PROCESS_TIMEOUT: Duration = Duration::from_secs(30);
const HELP: &str = "Usage: rusty-modbus-sim <CONFIG>\n\n\
                    Run a Modbus/TCP simulator from one validated YAML file.\n\n\
                    Options:\n  -h, --help  Print help\n";
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

struct TempConfig {
    path: PathBuf,
}

impl TempConfig {
    fn new(contents: &str) -> Self {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rusty-modbus-sim-{}-{sequence}.yaml",
            std::process::id()
        ));
        fs::write(&path, contents).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempConfig {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn valid_yaml(listen_addr: SocketAddr) -> String {
    format!(
        "device:\n  unit_id: 1\n  listen_addr: {listen_addr}\n\
         registers:\n  holding:\n    - address: 0\n      count: 2\n      initial: [48879]\n\
         faults: []\n"
    )
}

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-modbus-sim"));
    command.kill_on_drop(true);
    command
}

async fn output(args: &[&str]) -> Output {
    let mut command = command();
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn().unwrap();
    time::timeout(PROCESS_TIMEOUT, child.wait_with_output())
        .await
        .expect("simulator process timed out")
        .unwrap()
}

async fn next_stdout_line(lines: &mut Lines<BufReader<ChildStdout>>) -> String {
    time::timeout(PROCESS_TIMEOUT, lines.next_line())
        .await
        .expect("timed out waiting for simulator stdout")
        .unwrap()
        .expect("simulator stdout closed before the expected record")
}

fn parse_readiness(line: &str) -> (SocketAddr, u8) {
    let mut fields = line.split_whitespace();
    assert_eq!(fields.next(), Some("RUSTY_MODBUS_SIM_READY"));
    let address: SocketAddr = fields
        .next()
        .and_then(|field| field.strip_prefix("address="))
        .expect("readiness address field")
        .parse()
        .expect("readiness SocketAddr");
    let unit_id: u8 = fields
        .next()
        .and_then(|field| field.strip_prefix("unit_id="))
        .expect("readiness unit_id field")
        .parse()
        .expect("readiness unit ID");
    assert_eq!(fields.next(), None, "unexpected readiness field");
    assert_eq!(
        line,
        format!("RUSTY_MODBUS_SIM_READY address={address} unit_id={unit_id}")
    );
    (address, unit_id)
}

#[tokio::test]
async fn help_and_bad_arguments_have_stable_streams_and_exit_codes() {
    let help = output(&["--help"]).await;
    assert!(help.status.success());
    assert_eq!(help.stdout, HELP.as_bytes());
    assert!(help.stderr.is_empty());

    let missing = output(&[]).await;
    assert_eq!(missing.status.code(), Some(2));
    assert!(missing.stdout.is_empty());
    assert_eq!(
        String::from_utf8(missing.stderr).unwrap(),
        "rusty-modbus-sim: missing CONFIG\nTry 'rusty-modbus-sim --help' for usage.\n"
    );

    let extra = output(&["one.yaml", "two.yaml"]).await;
    assert_eq!(extra.status.code(), Some(2));
    assert!(extra.stdout.is_empty());
    assert!(
        String::from_utf8(extra.stderr)
            .unwrap()
            .contains("expected one CONFIG path")
    );
}

#[tokio::test]
async fn file_yaml_and_semantic_errors_exit_without_readiness() {
    let missing_path = std::env::temp_dir().join(format!(
        "rusty-modbus-sim-missing-{}.yaml",
        std::process::id()
    ));
    let _ = fs::remove_file(&missing_path);
    let missing = output(&[missing_path.to_str().unwrap()]).await;
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    assert!(
        String::from_utf8(missing.stderr)
            .unwrap()
            .contains("failed to read config")
    );

    let invalid_yaml = TempConfig::new("device: [\n");
    let invalid = output(&[invalid_yaml.path().to_str().unwrap()]).await;
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stdout.is_empty());
    assert!(
        String::from_utf8(invalid.stderr)
            .unwrap()
            .contains("config parse error")
    );

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let occupied_addr = listener.local_addr().unwrap();
    let unsupported = TempConfig::new(&format!(
        "device:\n  unit_id: 1\n  listen_addr: {occupied_addr}\n\
         registers:\n  holding:\n    - address: 0\n      count: 1\n      mode: increment\n\
         faults: []\n"
    ));
    let semantic = output(&[unsupported.path().to_str().unwrap()]).await;
    assert_eq!(semantic.status.code(), Some(1));
    assert!(semantic.stdout.is_empty());
    let stderr = String::from_utf8(semantic.stderr).unwrap();
    assert!(stderr.contains("only static is supported"), "{stderr}");
    assert!(!stderr.contains("Address already in use"), "{stderr}");
}

#[tokio::test]
async fn bind_failure_exits_without_readiness() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let config = TempConfig::new(&valid_yaml(listener.local_addr().unwrap()));

    let result = output(&[config.path().to_str().unwrap()]).await;

    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(
        stderr.contains("failed to start simulator: server error"),
        "{stderr}"
    );
    assert!(!stderr.contains("RUSTY_MODBUS_SIM_READY"));
}

#[tokio::test]
async fn port_zero_readiness_serves_a_request_and_shutdown_is_joined() {
    let config = TempConfig::new(&valid_yaml("127.0.0.1:0".parse().unwrap()));
    let mut command = command();
    let mut child = command
        .arg(config.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();

    let readiness = next_stdout_line(&mut lines).await;
    let (address, unit_id) = parse_readiness(&readiness);
    assert_ne!(address.port(), 0);
    assert_eq!(unit_id, 1);

    let client = ModbusClient::connect(
        address,
        ClientConfig {
            timeout: Duration::from_secs(5),
            ..ClientConfig::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        client
            .read_holding_registers(UnitId(unit_id), 0, 2)
            .await
            .unwrap(),
        vec![48879, 0]
    );
    #[cfg(unix)]
    {
        let status = Command::new("kill")
            .args(["-TERM", &child.id().unwrap().to_string()])
            .status()
            .await
            .unwrap();
        assert!(status.success());
        assert_eq!(
            next_stdout_line(&mut lines).await,
            "RUSTY_MODBUS_SIM_STOPPED"
        );
        assert!(
            time::timeout(PROCESS_TIMEOUT, child.wait())
                .await
                .expect("simulator did not exit after SIGTERM")
                .unwrap()
                .success()
        );
        assert_eq!(
            time::timeout(PROCESS_TIMEOUT, lines.next_line())
                .await
                .unwrap()
                .unwrap(),
            None,
            "simulator wrote an unexpected third stdout line"
        );
    }

    #[cfg(not(unix))]
    {
        child.kill().await.unwrap();
        let _ = child.wait().await;
    }

    client.shutdown().await;

    let mut diagnostics = String::new();
    stderr.read_to_string(&mut diagnostics).await.unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics}");
}
