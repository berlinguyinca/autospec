#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrowthReport {
    pub readme_ready: bool,
    pub demo_ready: bool,
    pub docs_ready: bool,
    pub issue_templates_ready: bool,
    pub launch_posts_ready: bool,
    pub launch_posts: Vec<String>,
    pub local_only: bool,
}

impl GrowthReport {
    pub fn new(
        readme_ready: bool,
        demo_ready: bool,
        docs_ready: bool,
        issue_templates_ready: bool,
        launch_posts_ready: bool,
        launch_posts: Vec<String>,
    ) -> Self {
        Self {
            readme_ready,
            demo_ready,
            docs_ready,
            issue_templates_ready,
            launch_posts_ready,
            launch_posts,
            local_only: true,
        }
    }

    pub fn ready_count(&self) -> usize {
        [
            self.readme_ready,
            self.demo_ready,
            self.docs_ready,
            self.issue_templates_ready,
            self.launch_posts_ready,
        ]
        .into_iter()
        .filter(|ready| *ready)
        .count()
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"command\":\"growth-report\",\"local_only\":{},\"ready_count\":{},\"readme_ready\":{},\"demo_ready\":{},\"docs_ready\":{},\"issue_templates_ready\":{},\"launch_posts_ready\":{},\"launch_posts\":{}}}",
            self.local_only,
            self.ready_count(),
            self.readme_ready,
            self.demo_ready,
            self.docs_ready,
            self.issue_templates_ready,
            self.launch_posts_ready,
            json_array(&self.launch_posts)
        )
    }

    pub fn to_markdown(&self) -> String {
        format!(
            "# AutoSpec Growth Report\n\nLocal only: {}\nReady checks: {}/5\n",
            self.local_only,
            self.ready_count()
        )
    }
}

fn json_array(values: &[String]) -> String {
    let values = values
        .iter()
        .map(|value| format!("\"{}\"", value.replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{values}]")
}
