use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub identity_cookie: String,
    pub fan_id: u64,
}

impl Credentials {
    pub fn new(identity_cookie: String, fan_id: u64) -> Self {
        Self {
            identity_cookie,
            fan_id,
        }
    }
}
