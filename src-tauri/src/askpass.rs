//! Bridges OpenSSH's password prompt to the app without putting secrets in files,
//! arguments or environment variables. A random, per-attempt token gates loopback IPC.
use crate::AppState;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use tauri::{Emitter, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::oneshot,
};
use uuid::Uuid;

pub const NO_PROMPT_ENV: &str = "GPU_MONITOR_NO_PROMPT";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    pub id: String,
    pub text: String,
    pub confirmation: bool,
}

pub struct Pending {
    pub prompt: Prompt,
    pub answer: oneshot::Sender<Option<String>>,
}

fn take_form_password(
    password: &mut Option<String>,
    expected_prompt: Option<&str>,
    prompt: &Prompt,
) -> Option<String> {
    if !prompt.confirmation && expected_prompt == Some(prompt.text.as_str()) {
        password.take()
    } else {
        None
    }
}

#[derive(Serialize, Deserialize)]
struct Request {
    token: String,
    prompt: String,
    confirmation: bool,
}

/// Entry point used only when this executable is invoked by OpenSSH as ASKPASS.
pub fn respond() -> i32 {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let address: std::net::SocketAddr = std::env::var("GPU_MONITOR_ASKPASS_ADDR")?.parse()?;
        if !address.ip().is_loopback() {
            return Err("nonlocal askpass endpoint".into());
        }
        let mut socket =
            std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_secs(5))?;
        socket.set_read_timeout(Some(std::time::Duration::from_secs(180)))?;
        let request = Request {
            token: std::env::var("GPU_MONITOR_ASKPASS_TOKEN")?,
            prompt: std::env::args().nth(1).unwrap_or_default(),
            confirmation: std::env::var("SSH_ASKPASS_PROMPT").as_deref() == Ok("confirm"),
        };
        writeln!(socket, "{}", serde_json::to_string(&request)?)?;
        let mut response = String::new();
        std::io::BufReader::new(socket)
            .take(65536)
            .read_line(&mut response)?;
        let answer: Option<String> = serde_json::from_str(&response)?;
        if let Some(answer) = answer {
            println!("{answer}");
            Ok(())
        } else {
            Err("cancelled".into())
        }
    })();
    if result.is_ok() {
        0
    } else {
        1
    }
}

pub struct Bridge {
    pub address: String,
    pub token: String,
    task: tokio::task::JoinHandle<()>,
    app: tauri::AppHandle,
}

impl Bridge {
    pub async fn new(
        app: tauri::AppHandle,
        mut initial_password: Option<String>,
        expected_password_prompt: Option<String>,
    ) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| e.to_string())?;
        let address = listener
            .local_addr()
            .map_err(|e| e.to_string())?
            .to_string();
        let token = Uuid::new_v4().to_string();
        let expected = token.clone();
        let handle = app.clone();
        let task = tokio::spawn(async move {
            let mut cancelled = false;
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut line = String::new();
                let read = async {
                    BufReader::new(&mut socket)
                        .take(16384)
                        .read_line(&mut line)
                        .await
                };
                if !matches!(
                    tokio::time::timeout(std::time::Duration::from_secs(5), read).await,
                    Ok(Ok(_))
                ) {
                    continue;
                }
                let Ok(request) = serde_json::from_str::<Request>(&line) else {
                    continue;
                };
                if request.token != expected {
                    continue;
                }
                if cancelled {
                    let _ = socket.write_all(b"null\n").await;
                    continue;
                }
                let prompt = Prompt {
                    id: Uuid::new_v4().to_string(),
                    confirmation: request.confirmation || request.prompt.contains("(yes/no"),
                    text: request.prompt,
                };
                // The caller verifies a direct connection; only its exact password prompt
                // may consume the form value, once. Other prompts remain explicit.
                if let Some(password) = take_form_password(
                    &mut initial_password,
                    expected_password_prompt.as_deref(),
                    &prompt,
                ) {
                    if let Ok(json) = serde_json::to_string(&Some(password)) {
                        let _ = socket.write_all(format!("{json}\n").as_bytes()).await;
                    }
                    continue;
                }
                let (answer, receive) = oneshot::channel();
                handle
                    .state::<AppState>()
                    .pending
                    .lock()
                    .unwrap()
                    .replace(Pending {
                        prompt: prompt.clone(),
                        answer,
                    });
                let _ = handle.emit("auth-prompt", Some(&prompt));
                let value = tokio::time::timeout(std::time::Duration::from_secs(175), receive)
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .flatten();
                cancelled = value.is_none();
                handle.state::<AppState>().pending.lock().unwrap().take();
                let _ = handle.emit("auth-prompt", Option::<Prompt>::None);
                if let Ok(json) = serde_json::to_string(&value) {
                    let _ = socket.write_all(format!("{json}\n").as_bytes()).await;
                }
            }
        });
        Ok(Self {
            address,
            token,
            task,
            app,
        })
    }

    pub fn configure(&self, cmd: &mut tokio::process::Command) -> Result<(), String> {
        cmd.env(
            "SSH_ASKPASS",
            std::env::current_exe().map_err(|e| e.to_string())?,
        )
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("DISPLAY", "gpu-monitor:0")
        .env("GPU_MONITOR_ASKPASS_ADDR", &self.address)
        .env("GPU_MONITOR_ASKPASS_TOKEN", &self.token)
        .env_remove(NO_PROMPT_ENV);
        Ok(())
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.task.abort();
        self.app.state::<AppState>().pending.lock().unwrap().take();
        let _ = self.app.emit("auth-prompt", Option::<Prompt>::None);
    }
}

// Import the Read traits for bounded line reads in both synchronous/asynchronous paths.
use std::io::Read;
use tokio::io::AsyncReadExt;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_password_is_bound_to_one_verified_prompt() {
        let expected = "alice@example.com's password: ";
        let mut password = Some("test-only-value".into());
        let prompt = |text: &str, confirmation| Prompt {
            id: "test".into(),
            text: text.into(),
            confirmation,
        };
        for text in [
            "alice@bastion.example.com's password: ",
            "Password:",
            "Enter passphrase for key 'password_key': ",
        ] {
            assert!(
                take_form_password(&mut password, Some(expected), &prompt(text, false)).is_none()
            );
        }
        assert!(take_form_password(&mut password, None, &prompt(expected, false)).is_none());
        assert!(
            take_form_password(&mut password, Some(expected), &prompt(expected, true)).is_none()
        );
        assert_eq!(
            take_form_password(&mut password, Some(expected), &prompt(expected, false)).as_deref(),
            Some("test-only-value")
        );
        assert!(
            take_form_password(&mut password, Some(expected), &prompt(expected, false)).is_none()
        );
    }
}
