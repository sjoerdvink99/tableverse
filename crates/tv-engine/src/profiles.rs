use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tv_core::Credentials;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub kind: String,
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub session_token: Option<String>,
    pub endpoint: Option<String>,
    pub region: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfilesConfig {
    pub profiles: HashMap<String, ConnectionProfile>,
}

pub fn load_profiles() -> ProfilesConfig {
    match std::fs::read_to_string(profile_path()) {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => ProfilesConfig::default(),
    }
}

pub fn resolve_credentials(profile_name: Option<&str>, inline: Option<Credentials>) -> Credentials {
    if let Some(creds) = inline {
        if creds.access_key.is_some() || creds.secret_key.is_some() {
            return creds;
        }
    }
    if let Some(name) = profile_name {
        let config = load_profiles();
        if let Some(profile) = config.profiles.get(name) {
            return Credentials {
                access_key: profile.access_key.clone(),
                secret_key: profile.secret_key.clone(),
                session_token: profile.session_token.clone(),
                endpoint: profile.endpoint.clone(),
                region: profile.region.clone(),
            };
        }
    }
    Credentials {
        access_key: std::env::var("AWS_ACCESS_KEY_ID").ok(),
        secret_key: std::env::var("AWS_SECRET_ACCESS_KEY").ok(),
        session_token: std::env::var("AWS_SESSION_TOKEN").ok(),
        endpoint: std::env::var("AWS_ENDPOINT_URL").ok(),
        region: std::env::var("AWS_DEFAULT_REGION").ok(),
    }
}

pub fn list_profile_names() -> Vec<String> {
    load_profiles().profiles.into_keys().collect()
}

fn profile_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".tableverse")
        .join("profiles.toml")
}
