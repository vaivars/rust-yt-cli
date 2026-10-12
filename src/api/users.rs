use serde::Deserialize;

use crate::api::{Client, Projection};
use crate::error::Result;

#[derive(Debug, Deserialize)]
pub struct Me {
    pub id: String,
    pub login: String,
    pub name: Option<String>,
}

impl Projection for Me {
    const FIELDS: &'static str = "id,login,name";
}

pub async fn me(client: &Client) -> Result<Me> {
    client.get("users/me", &[]).await
}
