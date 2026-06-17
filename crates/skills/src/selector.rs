//! SkillSelector — picks at most 3 skills from the loaded manifests based on
//! the ContextPlan's candidate list plus keyword matching against tags/description.

use crate::manifest::LoadedSkill;
use serde::{Deserialize, Serialize};

pub const MAX_SKILLS: usize = 3;
/// Max chars of a skill body to inject into the prompt.
pub const MAX_BODY_CHARS: usize = 1500;

/// A skill chosen for the current prompt turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedSkill {
    pub name: String,
    /// Truncated body text to inject into the prompt.
    pub body: String,
}

/// Select up to `MAX_SKILLS` skills for injection.
///
/// Selection order:
/// 1. Skills named in `candidate_names` (from ContextPlan).
/// 2. Skills matched by keyword in name/description/tags.
///
/// Skills that appear in both are deduplicated.
/// Only the bodies of selected skills are returned.
pub fn select(
    candidate_names: &[String],
    all_skills: &[LoadedSkill],
    user_turn: &str,
) -> Vec<SelectedSkill> {
    let low = user_turn.to_lowercase();

    // Phase 1: exact name matches from the plan
    let mut selected: Vec<SelectedSkill> = candidate_names
        .iter()
        .flat_map(|name| all_skills.iter().find(|s| &s.manifest.name == name))
        .map(to_selected)
        .collect();

    // Phase 2: keyword matching (stop once we hit MAX_SKILLS)
    if selected.len() < MAX_SKILLS {
        for skill in all_skills {
            if selected.len() >= MAX_SKILLS {
                break;
            }
            // Skip already selected
            if selected.iter().any(|s| s.name == skill.manifest.name) {
                continue;
            }
            if keyword_match(skill, &low) {
                selected.push(to_selected(skill));
            }
        }
    }

    selected.truncate(MAX_SKILLS);
    selected
}

fn to_selected(skill: &LoadedSkill) -> SelectedSkill {
    let body = if skill.body.len() > MAX_BODY_CHARS {
        let end = skill.body.floor_char_boundary(MAX_BODY_CHARS);
        format!("{}…", &skill.body[..end])
    } else {
        skill.body.clone()
    };
    SelectedSkill {
        name: skill.manifest.name.clone(),
        body,
    }
}

fn keyword_match(skill: &LoadedSkill, low_turn: &str) -> bool {
    let searchable = format!(
        "{} {} {}",
        skill.manifest.name,
        skill.manifest.description.to_lowercase(),
        skill.manifest.tags.join(" ").to_lowercase()
    );
    // Any word from the turn that appears in the skill's searchable text
    low_turn
        .split_whitespace()
        .any(|word| word.len() > 3 && searchable.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::parse_skill_content;

    fn make_skill(name: &str, desc: &str, tags: &[&str]) -> LoadedSkill {
        let yaml = format!(
            "---\nname: {name}\ndescription: {desc}\nversion: 1\nrisk: low\ntags:\n{}\n---\n\nBody of {name}.\n",
            tags.iter().map(|t| format!("  - {t}")).collect::<Vec<_>>().join("\n")
        );
        parse_skill_content(&yaml, "test".to_string()).unwrap()
    }

    fn make_all_skills() -> Vec<LoadedSkill> {
        vec![
            make_skill(
                "sk-business-email",
                "Use when drafting Slovak business emails",
                &["slovak", "email", "business"],
            ),
            make_skill(
                "mail-search",
                "Use for Apple Mail search and opening messages",
                &["mail", "email", "search"],
            ),
            make_skill(
                "invoice-analysis",
                "Use for invoices, DPH, faktúra, splatnosť",
                &["invoice", "dph", "accounting"],
            ),
            make_skill(
                "odoo-readonly",
                "Use for Odoo CRM lookups",
                &["odoo", "crm"],
            ),
            make_skill(
                "aerospace-window-control",
                "Use for AeroSpace workspace control",
                &["aerospace", "window", "workspace"],
            ),
        ]
    }

    #[test]
    fn selects_sk_business_email_by_name() {
        let all = make_all_skills();
        let selected = select(
            &["sk-business-email".to_string()],
            &all,
            "napíš formálnu odpoveď",
        );
        assert!(selected.iter().any(|s| s.name == "sk-business-email"));
    }

    #[test]
    fn selects_mail_search_by_name() {
        let all = make_all_skills();
        let selected = select(
            &["mail-search".to_string()],
            &all,
            "nájdi mail od Katky a otvor ho",
        );
        assert!(selected.iter().any(|s| s.name == "mail-search"));
    }

    #[test]
    fn selects_invoice_analysis_by_keyword() {
        let all = make_all_skills();
        // "invoice" keyword in the turn should match via keyword matching
        let selected = select(&[], &all, "check this invoice for payment terms");
        assert!(selected.iter().any(|s| s.name == "invoice-analysis"));
    }

    #[test]
    fn max_3_skills() {
        let all = make_all_skills();
        let candidates = vec![
            "sk-business-email".to_string(),
            "mail-search".to_string(),
            "invoice-analysis".to_string(),
            "odoo-readonly".to_string(),
        ];
        let selected = select(&candidates, &all, "test");
        assert!(selected.len() <= MAX_SKILLS);
    }

    #[test]
    fn no_duplicate_skills() {
        let all = make_all_skills();
        // Name in candidates + keyword match both point to same skill
        let selected = select(&["mail-search".to_string()], &all, "search mail inbox");
        let names: Vec<_> = selected.iter().map(|s| s.name.as_str()).collect();
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(names.len(), unique.len(), "duplicate skills in selection");
    }

    #[test]
    fn body_truncated_at_max_chars() {
        let long_body = "x".repeat(MAX_BODY_CHARS + 100);
        let yaml = format!(
            "---\nname: big-skill\ndescription: big\nversion: 1\nrisk: low\n---\n\n{long_body}"
        );
        let skill = parse_skill_content(&yaml, "test".to_string()).unwrap();
        let selected = select(&["big-skill".to_string()], &[skill], "test");
        assert!(selected[0].body.len() <= MAX_BODY_CHARS + 5); // +5 for ellipsis
    }
}
