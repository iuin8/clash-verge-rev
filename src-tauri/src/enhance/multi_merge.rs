use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictEntry {
    pub field: String,
    pub name: String,
    pub source: String,
    pub reason: String,
}

/// Merge an ordered list of YAML configs.
/// Only proxies/proxy-groups/rules are taken from supplementary profiles.
/// All other top-level keys come from primary (index 0).
pub fn multi_profile_merge(configs: &[Mapping], names: &[&str]) -> (Mapping, Vec<ConflictEntry>) {
    if configs.is_empty() {
        return (Mapping::new(), vec![]);
    }

    let mut base = configs[0].clone();
    let primary_name = names.first().copied().unwrap_or("primary");
    let mut conflicts: Vec<ConflictEntry> = vec![];

    for (i, supp) in configs[1..].iter().enumerate() {
        let supp_name = names.get(i + 1).copied().unwrap_or("unknown");

        // --- proxies ---
        if let Some(Value::Sequence(supp_proxies)) = supp.get("proxies") {
            let base_proxies = base
                .entry(Value::String("proxies".into()))
                .or_insert_with(|| Value::Sequence(vec![]));
            if let Value::Sequence(base_seq) = base_proxies {
                let existing_names: Vec<String> = base_seq
                    .iter()
                    .filter_map(|p| {
                        p.as_mapping()
                            .and_then(|m| m.get("name"))
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    })
                    .collect();

                let mut to_prepend: Vec<Value> = vec![];
                for p in supp_proxies.iter() {
                    let pname = p
                        .as_mapping()
                        .and_then(|m| m.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if existing_names.contains(&pname.to_string()) {
                        conflicts.push(ConflictEntry {
                            field: "proxies".into(),
                            name: pname.to_string(),
                            source: supp_name.to_string(),
                            reason: format!("already exists in {primary_name}"),
                        });
                    } else {
                        to_prepend.push(p.clone());
                    }
                }
                for item in to_prepend.into_iter().rev() {
                    base_seq.insert(0, item);
                }
            }
        }

        // --- proxy-groups ---
        if let Some(Value::Sequence(supp_groups)) = supp.get("proxy-groups") {
            let base_groups = base
                .entry(Value::String("proxy-groups".into()))
                .or_insert_with(|| Value::Sequence(vec![]));
            if let Value::Sequence(base_seq) = base_groups {
                let existing_group_names: Vec<String> = base_seq
                    .iter()
                    .filter_map(|g| {
                        g.as_mapping()
                            .and_then(|m| m.get("name"))
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    })
                    .collect();

                let mut groups_to_prepend: Vec<Value> = vec![];
                for g in supp_groups.iter() {
                    let gname = g
                        .as_mapping()
                        .and_then(|m| m.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !existing_group_names.contains(&gname.to_string()) {
                        groups_to_prepend.push(g.clone());
                    } else {
                        // Group exists: merge its member list
                        if let Some(existing) = base_seq.iter_mut().find(|eg| {
                            eg.as_mapping().and_then(|m| m.get("name")).and_then(|v| v.as_str()) == Some(gname)
                        }) && let Some(ex_map) = existing.as_mapping_mut()
                        {
                            let supp_members: Vec<Value> = g
                                .as_mapping()
                                .and_then(|m| m.get("proxies"))
                                .and_then(|v| v.as_sequence())
                                .cloned()
                                .unwrap_or_default();

                            let ex_members = ex_map
                                .entry(Value::String("proxies".into()))
                                .or_insert_with(|| Value::Sequence(vec![]));
                            if let Value::Sequence(ex_seq) = ex_members {
                                let existing_members: Vec<String> =
                                    ex_seq.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                                let mut members_to_prepend: Vec<Value> = vec![];
                                for m in supp_members.iter() {
                                    let mname = m.as_str().unwrap_or("");
                                    if !existing_members.contains(&mname.to_string()) {
                                        members_to_prepend.push(m.clone());
                                    }
                                }
                                for item in members_to_prepend.into_iter().rev() {
                                    ex_seq.insert(0, item);
                                }
                            }
                        }
                    }
                }
                for item in groups_to_prepend.into_iter().rev() {
                    base_seq.insert(0, item);
                }
            }
        }

        // --- rules ---
        if let Some(Value::Sequence(supp_rules)) = supp.get("rules") {
            let base_rules = base
                .entry(Value::String("rules".into()))
                .or_insert_with(|| Value::Sequence(vec![]));
            if let Value::Sequence(base_seq) = base_rules {
                let existing_rules: Vec<String> =
                    base_seq.iter().filter_map(|r| r.as_str().map(String::from)).collect();
                let mut rules_to_prepend: Vec<Value> = vec![];
                for r in supp_rules.iter() {
                    let rstr = r.as_str().unwrap_or("");
                    if !existing_rules.contains(&rstr.to_string()) {
                        rules_to_prepend.push(r.clone());
                    }
                }
                for item in rules_to_prepend.into_iter().rev() {
                    base_seq.insert(0, item);
                }
            }
        }

        // All other top-level keys from supplementary are intentionally ignored.
    }

    (base, conflicts)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn mapping(yaml: &str) -> Mapping {
        serde_yaml_ng::from_str(yaml).expect("test YAML is valid")
    }

    #[test]
    fn empty_configs_returns_empty_mapping() {
        let (result, conflicts) = multi_profile_merge(&[], &[]);
        assert!(result.is_empty());
        assert!(conflicts.is_empty());
    }

    #[test]
    fn single_config_returned_as_is() {
        let cfg = mapping("proxies:\n  - name: px1\n    type: ss");
        let (result, conflicts) = multi_profile_merge(std::slice::from_ref(&cfg), &["primary"]);
        assert_eq!(result, cfg);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn proxies_from_supplementary_are_prepended() {
        let primary = mapping("proxies:\n  - name: px-primary\n    type: ss");
        let supp = mapping("proxies:\n  - name: px-supp\n    type: vmess");
        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);
        assert!(conflicts.is_empty());
        let proxies = result.get("proxies").unwrap().as_sequence().unwrap();
        assert_eq!(proxies.len(), 2);
        assert_eq!(
            proxies[0].as_mapping().unwrap().get("name").unwrap().as_str().unwrap(),
            "px-supp"
        );
    }

    #[test]
    fn duplicate_proxy_name_logged_as_conflict() {
        let primary = mapping("proxies:\n  - name: dup\n    type: ss");
        let supp = mapping("proxies:\n  - name: dup\n    type: vmess");
        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].name, "dup");
        assert_eq!(conflicts[0].field, "proxies");
        let proxies = result.get("proxies").unwrap().as_sequence().unwrap();
        assert_eq!(proxies.len(), 1);
    }

    #[test]
    fn rules_from_supplementary_are_prepended_deduped() {
        let primary = mapping("rules:\n  - DOMAIN,example.com,DIRECT");
        let supp = mapping("rules:\n  - DOMAIN,example.com,DIRECT\n  - DOMAIN,other.com,PROXY");
        let (result, _conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);
        let rules = result.get("rules").unwrap().as_sequence().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].as_str().unwrap(), "DOMAIN,other.com,PROXY");
    }

    #[test]
    fn top_level_keys_from_primary_are_preserved() {
        let primary = mapping("dns:\n  enable: true\nproxies:\n  - name: p1\n    type: ss");
        let supp = mapping("dns:\n  enable: false\nproxies:\n  - name: p2\n    type: vmess");
        let (result, _) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);
        let dns = result.get("dns").unwrap().as_mapping().unwrap();
        assert!(dns.get("enable").unwrap().as_bool().unwrap());
    }
}
