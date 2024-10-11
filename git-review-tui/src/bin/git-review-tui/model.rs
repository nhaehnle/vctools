
#[derive(Debug)]
pub struct CodeReview {
    pub id: usize,
    pub target_branch: String,
}

pub type CodeReviewId = usize;

#[derive(Debug)]
pub struct Repository {
    pub name: Vec<String>,
    pub code_reviews: Vec<CodeReviewId>,
}

pub trait ForgeTrait {
    // fn get_repositories(&self) -> Vec<Repository>;
}

pub enum Forge {
    GitHub(crate::github::GitHubForge),
}
impl Forge {
    pub fn close(self) {
        match self {
            Forge::GitHub(forge) => forge.close(),
        }
    }
}
impl std::ops::Deref for Forge {
    type Target = dyn ForgeTrait;

    fn deref(&self) -> &Self::Target {
        match self {
            Forge::GitHub(forge) => forge,
        }
    }
}
impl std::ops::DerefMut for Forge {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Forge::GitHub(forge) => forge,
        }
    }
}
