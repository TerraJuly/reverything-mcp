use std::{io, process::ExitStatus};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

const MAX_STDOUT: usize = 4 * 1024 * 1024;
const MAX_STDERR: usize = 64 * 1024;

pub struct CapturedOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

async fn read_bounded(reader: impl AsyncRead + Unpin, cap: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(cap as u64 + 1).read_to_end(&mut bytes).await?;
    if bytes.len() > cap {
        return Err(io::Error::other(
            "ES output exceeded the configured safety limit",
        ));
    }
    Ok(bytes)
}

pub async fn run_limited(command: &mut Command) -> io::Result<CapturedOutput> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout configured as piped");
    let stderr = child.stderr.take().expect("stderr configured as piped");
    let result = tokio::try_join!(
        child.wait(),
        read_bounded(stdout, MAX_STDOUT),
        read_bounded(stderr, MAX_STDERR),
    );
    match result {
        Ok((status, stdout, stderr)) => Ok(CapturedOutput {
            status,
            stdout,
            stderr,
        }),
        Err(error) => {
            // On a pipe/size failure, terminate and reap instead of leaving ES
            // running after its output readers have been discarded.
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn accepts_output_at_limit() {
        assert_eq!(read_bounded(&b"abcd"[..], 4).await.unwrap(), b"abcd");
        assert!(read_bounded(&b"abcde"[..], 4).await.is_err());
        assert!(read_bounded(&b""[..], 4).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn drains_stdout_and_stderr_concurrently() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--ignored",
            "--exact",
            "process::tests::subprocess_fixture",
            "--nocapture",
        ]);
        command.env("REVERYTHING_TEST_CHILD", "output");
        let output = run_limited(&mut command).await.unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("fixture-stdout"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("fixture-stderr"));
    }

    #[tokio::test]
    async fn timeout_drops_a_running_child() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--ignored",
            "--exact",
            "process::tests::subprocess_fixture",
            "--nocapture",
        ]);
        command.env("REVERYTHING_TEST_CHILD", "wait");
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            run_limited(&mut command),
        )
        .await;
        assert!(result.is_err());
    }

    #[test]
    #[ignore = "helper invoked only by subprocess tests"]
    fn subprocess_fixture() {
        match std::env::var("REVERYTHING_TEST_CHILD").as_deref() {
            Ok("output") => {
                println!("fixture-stdout");
                eprintln!("fixture-stderr");
            }
            Ok("wait") => std::thread::sleep(std::time::Duration::from_secs(30)),
            _ => {}
        }
    }
}
