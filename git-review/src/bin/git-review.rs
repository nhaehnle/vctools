// SPDX-License-Identifier: GPL-3.0-or-later

use clap::Parser;

use diff_modulo_base::*;
use directories::ProjectDirs;
use reqwest::header;
use serde::Deserialize;
use utils::{Result, try_forward};

use git_core::{Ref, Repository};

mod github {
    use serde::Deserialize;

    #[derive(Deserialize, Debug)]
    pub struct Branch {
        #[serde(rename = "ref")]
        pub ref_: String,
        pub sha: String,
    }

    #[derive(Deserialize, Debug)]
    pub struct Pull {
        pub head: Branch,
        pub base: Branch,
    }

    #[derive(Deserialize, Debug)]
    pub struct User {
        pub login: String,
    }

    #[derive(Deserialize, Debug)]
    pub struct Review {
        pub user: User,
        pub commit_id: String,
    }
}

#[derive(Deserialize, Debug)]
struct Host {
    host: String,
    api: String,
    user: String,
    token: String,
}

#[derive(Deserialize, Debug)]
struct Config {
    hosts: Vec<Host>,
}

#[derive(Parser, Debug)]
struct Cli {
    remote: String,
    pull: i32,

    #[clap(flatten)]
    dmb_options: tool::GitDiffModuloBaseOptions,

    /// Behave as if run from the given path.
    #[clap(short = 'C', default_value = ".")]
    path: std::path::PathBuf,

    #[clap(flatten)]
    cli_options: cli::Options,
}

trait JsonRequest {
    fn send_json<'a, J>(self) -> Result<J>
    where J: serde::de::DeserializeOwned;
}
impl JsonRequest for reqwest::blocking::RequestBuilder {
    fn send_json<'a, J>(self) -> Result<J>
    where J: serde::de::DeserializeOwned {
        let (client, request) = self.build_split();
        let request = request?;
        let request_clone = request.try_clone();

        try_forward(
            move || -> Result<J> {
                let response = client.execute(request)?;
                if !response.status().is_success() {
                    Err(format!("HTTP error: {}", response.status()))?
                }

                let body = response.text()?;
                match serde_json::from_str(&body) {
                    Ok(json) => Ok(json),
                    Err(err) => Err(format!("Error parsing JSON: {err}\n{body}\n"))?,
                }
            },
            || format!("Error processing request: {request_clone:?}"),
        )
    }
}

fn do_main() -> Result<()> {
    let args = Cli::parse();
    let mut cli = cli::Cli::new(args.cli_options);
    let out = cli.stream();

    let dirs = ProjectDirs::from("experimental", "nhaehnle", "diff-modulo-base").unwrap();
    let config: Config = {
        let mut config = dirs.config_dir().to_path_buf();
        config.push("github.toml");
        try_forward(
            || Ok(toml::from_str(&String::from_utf8(utils::read_bytes(config)?)?)?),
            || "Error parsing configuration",
        )?
    };

    //    println!("{:?}", &config);
    //    println!("{}", dirs.config_dir().display());

    let git_repo = Repository::new(&args.path);
    let url = git_repo.get_url(&args.remote)?;
    let Some(hostname) = url.hostname() else {
        Err("remote is local")?
    };
    let Some((organization, gh_repo)) = url.github_path() else {
        Err(format!("cannot parse {url} as a GitHub repository"))?
    };

    let Some(host) = config.hosts.iter().find(|host| host.host == hostname) else {
        print!("Host {hostname} not found in config");
        Err("host not configured")?
    };

    let mut default_headers = header::HeaderMap::new();
    default_headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {}", host.token).parse()?,
    );
    default_headers.insert(header::ACCEPT, "application/vnd.github+json".parse()?);
    default_headers.insert("X-GitHub-Api-Version", "2022-11-28".parse()?);

    let client = reqwest::blocking::Client::builder()
        .user_agent("git-review")
        .default_headers(default_headers)
        .build()?;

    let url_api = reqwest::Url::parse(&host.api)?;
    let url_api_repo = url_api.join(format!("repos/{organization}/{gh_repo}/").as_str())?;

    let url_api_pull = url_api_repo.join(format!("pulls/{}", args.pull).as_str())?;
    let url_api_reviews = url_api_repo.join(format!("pulls/{}/reviews", args.pull).as_str())?;

    let pull: github::Pull = client.get(url_api_pull).send_json()?;
    let reviews: Vec<github::Review> = client.get(url_api_reviews).send_json()?;

    let most_recent_review = reviews
        .into_iter()
        .rev()
        .find(|review| review.user.login == host.user);

    writeln!(out, "Review {}/{}#{}", organization, gh_repo, args.pull)?;
    if let Some(review) = &most_recent_review {
        writeln!(out, "  Most recent review: {}", review.commit_id)?;
    }
    writeln!(out, "  Current head:       {}", pull.head.sha)?;
    writeln!(out, "  Target branch:      {}", pull.base.ref_)?;

    let refs: Vec<_> = [&pull.head.sha, &pull.base.sha]
        .into_iter()
        .chain(most_recent_review.iter().map(|review| &review.commit_id))
        .map(|sha| Ref::new(sha))
        .collect();
    git_repo.fetch_missing(&args.remote, &refs)?;

    let old = if let Some(review) = most_recent_review {
        review.commit_id
    } else {
        git_repo
            .merge_base(&Ref::new(&pull.base.sha), &Ref::new(&pull.head.sha))?
            .name
    };

    let dmb_args = tool::GitDiffModuloBaseArgs {
        base: Some(pull.base.sha),
        old: Some(old),
        new: Some(pull.head.sha),
        options: args.dmb_options,
    };

    let mut writer = diff_color::Writer::new();
    tool::git_diff_modulo_base(dmb_args, git_repo, &mut writer)?;
    writer.write(out)?;

    Ok(())
}

fn main() {
    if let Err(err) = do_main() {
        println!("{}", err);
        std::process::exit(1);
    }
}
