use anyhow::{anyhow, Context, Result};
use basert_connector::{BaseRtClient, DEFAULT_BASE_URL};
use std::time::Duration;

const BAGENT_BASERT_LAUNCH_AGENT: &str = "com.bagent.basert";

async fn managed_pid(uid: &str) -> Option<u32> {
    let target = format!("gui/{uid}/{BAGENT_BASERT_LAUNCH_AGENT}");
    let output = tokio::process::Command::new("/bin/launchctl")
        .args(["print", &target])
        .output()
        .await
        .ok()?;
    output.status.success().then_some(())?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().strip_prefix("pid = "))
        .and_then(|value| value.parse::<u32>().ok())
}

pub(crate) async fn restart_managed_basert(client: &BaseRtClient) -> Result<()> {
    if client.endpoint() != DEFAULT_BASE_URL {
        return Err(anyhow!(
            "BaseRT runtime restart is unsupported for this endpoint"
        ));
    }
    let uid = tokio::process::Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .await
        .context("resolve user id for BaseRT restart")?;
    if !uid.status.success() {
        return Err(anyhow!("resolve user id for BaseRT restart"));
    }
    let uid = String::from_utf8_lossy(&uid.stdout).trim().to_string();
    let previous_pid = managed_pid(&uid).await;
    let target = format!("gui/{uid}/{BAGENT_BASERT_LAUNCH_AGENT}");
    let restart = tokio::process::Command::new("/bin/launchctl")
        .args(["kickstart", "-k", &target])
        .output()
        .await
        .context("restart poisoned BaseRT runtime")?;
    if !restart.status.success() {
        return Err(anyhow!("restart poisoned BaseRT runtime failed"));
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let current_pid = managed_pid(&uid).await;
        let new_process = current_pid.is_some() && current_pid != previous_pid;
        if new_process && client.is_up().await {
            let models = client.inspect_models().await?;
            if models.iter().all(|model| !model.loaded) {
                client.clear_runtime_fault();
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!("restarted BaseRT did not become clean"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
