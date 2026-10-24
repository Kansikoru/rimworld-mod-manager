use crate::models::CommunityRule;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;
use std::time::Duration;

const DEFAULT_RULES_JSON: &str = include_str!("../data/default_community_rules.json");
static DEFAULT_DB: OnceLock<CommunityDatabase> = OnceLock::new();

/// Возвращает ссылку на встроенную базу правил сообщества по умолчанию
pub fn get_default_community_db() -> &'static CommunityDatabase {
    DEFAULT_DB.get_or_init(|| {
        let parsed: HashMap<String, CommunityRule> = serde_json::from_str(DEFAULT_RULES_JSON)
            .unwrap_or_default();
        let mut lowercased_rules = HashMap::with_capacity(parsed.len());
        for (k, v) in parsed {
            lowercased_rules.insert(k.to_lowercase(), v);
        }
        CommunityDatabase {
            rules: lowercased_rules,
        }
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommunityDatabase {
    pub rules: HashMap<String, CommunityRule>,
}

pub struct DatabaseManager {
    db: CommunityDatabase,
    cache_path: PathBuf,
}

impl DatabaseManager {
    pub fn new<P: AsRef<Path>>(cache_path: P) -> Self {
        Self {
            db: get_default_community_db().clone(),
            cache_path: cache_path.as_ref().to_path_buf(),
        }
    }

    pub fn load_local(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.cache_path.exists() {
            let data = fs::read_to_string(&self.cache_path)?;
            let parsed: HashMap<String, CommunityRule> = serde_json::from_str(&data)?;
            
            let mut lowercased_rules = get_default_community_db().rules.clone();
            for (k, v) in parsed {
                lowercased_rules.insert(k.to_lowercase(), v);
            }
            self.db.rules = lowercased_rules;
        } else {
            self.db = get_default_community_db().clone();
        }
        Ok(())
    }

    pub fn get_db(&self) -> &CommunityDatabase {
        &self.db
    }

    pub fn fetch_remote_rules(
        &mut self,
        repo_url: &str,
        progress_tx: Option<Sender<String>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let _ = progress_tx.as_ref().map(|tx| tx.send("Подключение к серверу...".to_string()));

        let mut request = ureq::get(repo_url).timeout(Duration::from_secs(10));
        
        let etag_path = self.cache_path.with_extension("etag");
        if etag_path.exists() {
            if let Ok(etag) = fs::read_to_string(&etag_path) {
                request = request.set("If-None-Match", &etag);
            }
        }

        let _ = progress_tx.as_ref().map(|tx| tx.send("Проверка обновлений...".to_string()));

        let response = match request.call() {
            Ok(res) => res,
            Err(ureq::Error::Status(304, _)) => {
                let _ = progress_tx.as_ref().map(|tx| tx.send("База правил не требует обновления.".to_string()));
                return self.load_local();
            },
            Err(e) => return Err(Box::new(e)),
        };

        let remote_etag = response.header("ETag").map(|s| s.to_string());
        
        let _ = progress_tx.as_ref().map(|tx| tx.send("Загрузка данных...".to_string()));
        let text = response.into_string()?;
        
        let _ = progress_tx.as_ref().map(|tx| tx.send("Парсинг правил...".to_string()));
        let parsed: HashMap<String, CommunityRule> = serde_json::from_str(&text)?;
        
        let mut lowercased_rules = get_default_community_db().rules.clone();
        for (k, v) in parsed {
            lowercased_rules.insert(k.to_lowercase(), v);
        }
        
        let _ = progress_tx.as_ref().map(|tx| tx.send("Сохранение базы...".to_string()));
        let tmp_path = self.cache_path.with_extension("tmp");
        fs::write(&tmp_path, &text)?;
        fs::rename(&tmp_path, &self.cache_path)?;

        if let Some(etag) = remote_etag {
            fs::write(etag_path, etag)?;
        }
        
        self.db.rules = lowercased_rules;
        let _ = progress_tx.as_ref().map(|tx| tx.send("Обновление завершено.".to_string()));

        Ok(())
    }
}
