
#[derive(Debug)]
pub struct CodeReview {
    pub id: usize,
    pub target_branch: String,
}

#[derive(Debug)]
pub struct Repository {
    pub name: Vec<String>,
    pub code_reviews: Vec<CodeReview>,
}

pub trait Forge {
    fn get_repositories(&self) -> Vec<Repository>;
}
