use super::entry::Frontmatter;

pub(crate) fn parse_frontmatter(body: &str) -> Frontmatter {
    let mut lines = body.lines();
    if lines.next() != Some("---") {
        return Frontmatter::default();
    }

    let mut frontmatter = Frontmatter::default();
    while let Some(line) = lines.next() {
        if line == "---" {
            break;
        }
        if let Some(raw) = line.strip_prefix("name:") {
            frontmatter.name = Some(unquote(raw.trim()));
        } else if let Some(raw) = line.strip_prefix("description:") {
            let raw = raw.trim();
            if raw == ">" || raw == ">-" || raw == "|" || raw == "|-" {
                let mut parts = Vec::new();
                for next in lines.by_ref() {
                    if next == "---" {
                        break;
                    }
                    if !next.starts_with(' ') && next.contains(':') {
                        break;
                    }
                    let trimmed = next.trim();
                    if !trimmed.is_empty() {
                        parts.push(trimmed);
                    }
                }
                frontmatter.description = Some(parts.join(" "));
                break;
            }
            frontmatter.description = Some(unquote(raw));
        }
    }
    frontmatter
}

fn unquote(raw: &str) -> String {
    raw.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_frontmatter() {
        let frontmatter =
            parse_frontmatter("---\nname: alpha\ndescription: Alpha skill\n---\n\n# Body\n");
        assert_eq!(frontmatter.name.as_deref(), Some("alpha"));
        assert_eq!(frontmatter.description.as_deref(), Some("Alpha skill"));
    }

    #[test]
    fn parses_folded_description_frontmatter() {
        let frontmatter = parse_frontmatter(
            "---\nname: alpha\ndescription: >-\n  First line\n  second line\n---\n",
        );
        assert_eq!(
            frontmatter.description.as_deref(),
            Some("First line second line")
        );
    }
}
