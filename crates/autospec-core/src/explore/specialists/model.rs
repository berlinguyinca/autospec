#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecialistRoster {
    pub schema_version: u8,
    pub domains: Vec<DetectedDomain>,
    pub suggested_specialists: Vec<SuggestedSpecialist>,
}

impl SpecialistRoster {
    pub fn capped(mut self, limit: usize) -> Self {
        self.suggested_specialists.truncate(limit.min(6));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedDomain {
    pub name: String,
    pub score: usize,
    pub evidence: Vec<FileLineEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLineEvidence {
    pub file: String,
    pub line: usize,
    pub r#match: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestedSpecialist {
    pub slug: String,
    pub persona: String,
    pub lens: String,
    pub why: String,
    pub evidence: String,
}

pub(crate) fn normalized_slug(value: &str) -> Option<String> {
    let mut slug = String::new();
    let mut needs_separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if needs_separator && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character.to_ascii_lowercase());
            needs_separator = false;
        } else {
            needs_separator = true;
        }
    }
    (!slug.is_empty()).then_some(slug)
}
