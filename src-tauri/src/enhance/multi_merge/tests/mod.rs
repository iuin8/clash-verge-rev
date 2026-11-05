use super::Mapping;

mod core;
mod groups;
mod malformed;

fn mapping(yaml: &str) -> Mapping {
    serde_yaml_ng::from_str(yaml).expect("test YAML is valid")
}

/// 注入占位符 proxy/provider 定义，让 group 测试里的 `a`/`b`/`provider-a` 引用真实存在，
/// 避免层 3 引用完整性校验把它们当作无效引用移除。
fn group_mapping(yaml: &str) -> Mapping {
    mapping(&format!(
        "proxies:\n  - name: a\n    type: ss\n  - name: b\n    type: ss\nproxy-providers:\n  provider-a:\n    type: http\n    url: https://a.example\n  provider-b:\n    type: http\n    url: https://b.example\n{yaml}"
    ))
}
