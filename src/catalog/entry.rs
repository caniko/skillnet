use camino::Utf8PathBuf;

#[derive(Debug, Clone)]
pub(crate) struct SkillEntry {
    pub(crate) name: String,
    pub(crate) qualified_name: String,
    pub(crate) scope: String,
    pub(crate) project: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) status: String,
    pub(crate) tags: Vec<String>,
    pub(crate) related_skills: Vec<String>,
    pub(crate) collision_note: Option<String>,
    pub(crate) description: String,
    pub(crate) path: Utf8PathBuf,
    pub(crate) line_count: usize,
}

#[derive(Debug, Default)]
pub(crate) struct Frontmatter {
    pub(crate) name: Option<String>,
    pub(crate) description: Option<String>,
}

impl SkillEntry {
    pub(super) fn matches(&self, query: &str) -> bool {
        let fields = [
            self.name.as_str(),
            self.qualified_name.as_str(),
            self.project.as_deref().unwrap_or_default(),
            self.category.as_deref().unwrap_or_default(),
            self.status.as_str(),
            self.description.as_str(),
        ];
        fields
            .iter()
            .any(|field| field.to_lowercase().contains(query))
            || self
                .tags
                .iter()
                .any(|tag| tag.to_lowercase().contains(query))
    }
}
