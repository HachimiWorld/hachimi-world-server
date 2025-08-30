use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_yaml::Value;
use std::fs;
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct Config {
    value: Arc<Value>,
}

impl Config {
    pub fn parse(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let content = fs::read_to_string(&path).with_context(|| format!("Failed to read config file: {:?}", path.as_ref()))?;
        Self::parse_by_str(&content)
    }

    pub fn parse_by_str(str: &str) -> anyhow::Result<Self> {
        let value = serde_yaml::from_str::<Value>(str)?;
        Ok(Config { value: Arc::new(value) })
    }

    pub fn get(&self, key: &str) -> anyhow::Result<Option<&Value>> {
        let result = key.split('.').fold(Some(self.value.deref()), |value, key| {
            if let Some(value) = value {
                value.get(key)
            } else {
                None
            }
        });
        Ok(result)
    }

    pub fn get_and_parse<T>(&self, key: &str) -> anyhow::Result<T>
    where
        T: DeserializeOwned,
    {
        let value = self
            .get(key)?
            .cloned()
            .with_context(|| format!("Config [{key}] does not exists"))?;
        let config: T = serde_yaml::from_value(value)
            .with_context(|| format!("Failed to parse config with key: {key}"))?;
        Ok(config)
    }

    pub fn get_str(&self, key: &str) -> anyhow::Result<Option<String>> {
        let value = self.get(key)?;
        Ok(value.and_then(|v| {
            if v.is_number() {
                v.as_i64().and_then(|x| Some(x.to_string()))
            } else {
                Some(v.as_str()?.to_string())
            }
        }))
    }
    pub fn get_num(&self, key: &str) -> anyhow::Result<Option<i64>> {
        let value = self.get(key)?;
        Ok(value.and_then(|v| v.as_i64()))
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use serde::Deserialize;

    const TEST_CONFIG: &str = r#"
server:
  port: 8080
postgres:
  host: 127.0.0.1
  port: 5432
  user: postgres
  password: postgres
"#;
    #[test]
    fn test_parse_and_get() {
        let cfg = Config::parse_by_str(TEST_CONFIG).unwrap();
        assert_eq!(Some(8080), cfg.get_num("server.port").unwrap());
        assert_eq!(
            Some("127.0.0.1".to_string()),
            cfg.get_str("postgres.host").unwrap()
        );
    }

    #[test]
    fn test_get_and_parse() {
        #[derive(Deserialize)]
        struct PostgresCfg {
            host: String,
            port: i64,
            user: String,
            password: String,
        }
        let cfg = Config::parse_by_str(TEST_CONFIG).unwrap();
        let data = cfg.get_and_parse::<PostgresCfg>("postgres").unwrap();
        assert_eq!("127.0.0.1", data.host)
    }
}
