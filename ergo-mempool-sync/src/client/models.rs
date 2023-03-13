#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiInfo {
    pub full_height: u32,
}
