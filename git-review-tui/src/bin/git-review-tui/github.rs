
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubAccount {
    url: String,
    user: String,
    token: String,
}

#[derive(Debug)]
pub struct GitHubForge {

}
impl GitHubForge {
    pub fn open(account: GitHubAccount) -> Self {
        Self {}
    }
}
