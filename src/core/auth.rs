use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub identity_cookie: String,
    pub client_id: Option<String>,
    pub fan_id: Option<String>,
}

impl Credentials {
    pub fn new(identity_cookie: String) -> Self {
        Self {
            identity_cookie,
            client_id: None,
            fan_id: None,
        }
    }
}
