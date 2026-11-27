use super::*;
use uuid::Uuid;

// One file per successful operation. Independent writes and the server's unique key
// make overlapping windows and lost HTTP responses safe to retry.
pub fn queue_install(data: &Path, card: Uuid) -> Result<()> {
    let dir = core::no_links(&data.join("recommendations").join("pending-installs"))?;
    fs::create_dir_all(&dir)?;
    let operation = Uuid::new_v4();
    core::atomic_json(&dir.join(format!("{operation}.json")), &card.to_string())?;
    if let Ok(client) = Client::new(data) {
        client.retry_installs();
    }
    Ok(())
}

static SENDING: Mutex<()> = Mutex::new(());
impl Client {
    pub fn retry_installs(&self) {
        let dir = self.root.join("pending-installs");
        if !dir.is_dir() {
            return;
        }
        let client = self.clone();
        std::thread::spawn(move || {
            let Ok(_guard) = SENDING.try_lock() else {
                return;
            };
            client.flush_installs(&dir);
        });
    }

    fn flush_installs(&self, dir: &Path) {
        let Ok(dir) = core::no_links(dir) else {
            return;
        };
        let Ok(files) = fs::read_dir(dir) else {
            return;
        };
        for item in files.flatten() {
            let path = item.path();
            if path.extension().is_none_or(|s| s != "json")
                || !item.file_type().is_ok_and(|f| f.is_file())
            {
                continue;
            }
            let Some(operation) = path
                .file_stem()
                .and_then(|n| n.to_str())
                .and_then(|n| Uuid::parse_str(n).ok())
            else {
                continue;
            };
            let Some(card) = core::read_json(&path, 128)
                .ok()
                .and_then(|v| Uuid::parse_str(v.as_str()?).ok())
            else {
                continue;
            };
            let result = self.request::<Value>(
                Method::POST,
                &format!("/v1/recipes/{card}/installs"),
                Some(json!({"operation":operation.to_string()})),
                false,
            );
            match result {
                Ok(v) if v["ok"] == true => {
                    let _ = fs::remove_file(path);
                }
                Err(err)
                    if matches!(
                        err.downcast_ref::<ServiceError>(),
                        Some(ServiceError::Api("统计目标已移除"))
                    ) =>
                {
                    let _ = fs::remove_file(path);
                }
                _ => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{mock_service, online_test_client};
    use super::*;

    #[test]
    fn pending_install_survives_an_old_server_and_reuses_its_receipt() -> Result<()> {
        let data = tempfile::tempdir()?;
        let game_card = Uuid::new_v4();
        let operation = Uuid::new_v4();
        let pending = data.path().join("recommendations/pending-installs");
        fs::create_dir_all(&pending)?;
        let receipt = pending.join(format!("{operation}.json"));
        core::atomic_json(&receipt, &game_card.to_string())?;
        for (status, body, should_remain) in [
            (404, json!({"error":"not_found"}), true),
            (200, json!({"ok":true}), false),
        ] {
            let (origin, server) = mock_service(vec![(status, body)]);
            let client = online_test_client(data.path(), origin);
            client.flush_installs(&pending);
            let requests = server.join().unwrap();
            assert_eq!(requests.len(), 1);
            assert!(requests[0].contains(&format!("POST /v1/recipes/{game_card}/installs ")));
            assert!(requests[0].contains(&operation.to_string()));
            assert_eq!(receipt.exists(), should_remain);
        }
        Ok(())
    }
}
