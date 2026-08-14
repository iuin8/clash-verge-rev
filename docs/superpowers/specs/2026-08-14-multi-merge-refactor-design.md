# multi_merge 重构设计

日期：2026-08-14
状态：待 review

## 背景与目标

fork 的 `enhance/multi_merge.rs` 实现了多订阅字段级合并（1154 行 + 8 个测试）。近期排查发现三类问题，本设计一次性解决：

1. **脏数据**：删除 profile 时 `merged` 列表未同步清理，留下失效 uid → 每次校验失败（已修复容错，本设计治本）。
2. **静默错误**：`merge_proxies` 仅按 `name` 去重，不比较定义；`merge_proxy_groups` 不校验成员存在性。两者叠加导致「group 静默指向 primary 的同名但不同定义 proxy」。
3. **合并语义过粗**：非合并顶层字段（`dns` 等）整块 primary 赢，无法按子字段合并。

目标：**结构性消除静默错误 + 保持行为可预测 + 低风险交付**。

## 范围

**做**：
- 阶段 1：`delete_item` 同步清理 `merged`（治本）。
- 阶段 2：`multi_merge.rs` 重构为声明式管线，引入跨字段「引用完整性」三层机制。
- 阶段 3：对**自包含 mapping 字段**引入 deep merge（递归合并）。

**不做**（已确认延后/取消）：
- 阶段 4 声明式重命名/过滤（`PrfOption` 加 `filter`/`rename` + 前端 UI）：与多订阅合并无直接关联，改动牵前端+上游，暂缓。
- 不抽公共库：单一消费方，YAGNI。
- 不动 mihomo 内核。

## 关键决策（已确认）

| # | 决策 | 结论 |
|---|------|------|
| 1 | 重名不同定义 proxy 的重命名后缀 | `X [supp_name]`（可读、标明来源） |
| 2 | 引用完整性校验失败的行为 | 记 conflict + 降级不合并该引用，**不阻断启动** |
| 3 | 定义比较忽略的字段 | 仅忽略 `name`，其余全比 |

## 阶段 1：治本（清理 merged 脏数据）

**改动点**：`config/profiles.rs::delete_item`（341-396）。

删除 profile 后，从 `self.merged` 中 `retain` 掉被删 uid 及 `delete_uids`（关联的 merge/script/rules/proxies/groups）。

```rust
// 删除主 item 与关联 item 之后，同步清理 merged
if let Some(mut merged) = self.merged.take() {
    let removed: HashSet<&String> = std::iter::once(uid).chain(delete_uids.iter()).collect();
    merged.retain(|u| !removed.contains(u));
    if !merged.is_empty() {
        self.merged = Some(merged);
    } else {
        self.merged = None;
    }
}
```

**验证**：单测「删除 merged 中的 profile 后，`merged` 不再含该 uid 及其关联 uid」。

## 阶段 2：声明式管线 + 引用完整性

### 2.1 声明式管线

将 `multi_profile_merge` 的硬编码 6 步调用改为声明式步骤列表：

```rust
struct MergeContext {
    primary_name: String,
    conflicts: Vec<ConflictEntry>,
    renames: HashMap<String, String>, // 旧名 -> 新名（层 2 产出，覆盖 proxies/providers/rule-providers）
}

type MergeStep = fn(&mut MergeContext, &mut Mapping, &Mapping, &str, &str);

const STEPS: &[MergeStep] = &[
    merge_proxies,          // 产出 renames
    merge_proxy_providers,  // 产出 renames
    merge_proxy_groups,     // 消费 renames 重写 proxies/use 引用
    merge_rules,            // 消费 renames 重写 RULE-SET 引用
    merge_rule_providers,   // 产出 renames
    log_top_level_key_conflicts,
];
```

循环对每个 supplementary 依次执行 STEPS。行为等价，顺序可配置、可测试。

### 2.2 三层引用完整性

#### 层 1：定义比较去重（named 合并：proxies / proxy-providers / rule-providers）

重名时不再无条件丢弃 supp 版，而是比较**除 `name` 外的完整定义**：

- 定义相同 → 真重复，安全丢弃。
- 定义不同 → 进入层 2（不丢）。

#### 层 2：重命名保底 + 引用重写（三类统一）

定义不同的 supp 项重命名为 `X [supp_name]`，记录 `renames[X] = X [supp_name]`。消费方合并时经映射重写引用：

| 重命名来源 | 重写消费方 |
|-----------|-----------|
| proxies | group 的 `proxies` 成员 |
| proxy-providers | group 的 `use` |
| rule-providers | rules 的 `RULE-SET,<名>,...` |

```yaml
# base:   X (server=1.2.3.4)
# supp:   X (server=5.6.7.8) + group G 引用 X
# 结果:   proxies 含 X 与 "X [supp_name]"；G 引用 "X [supp_name]"
```

三类走同一 `renames` 结构 + 各自消费点，不是三套代码。

#### 层 3：引用完整性校验（兜底）

合并全部完成后，扫描所有 group 的 `proxies`/`use` 成员，凡是不在最终 config 的 proxies/providers/groups 集合中的，记 `ConflictEntry` 并**从该 group 成员中移除**（降级不合并，不阻断启动）。

覆盖四类跨字段引用：proxies↔groups、providers↔group `use`、rules→group、group 互引。

**为什么降级而非硬报错**：硬报错会把脏数据重新变成「每次校验失败」的老问题；降级 + conflict 记录，既不影响启动，又让用户在日志/UI 可见。

## 阶段 3：deep merge（A 类自包含字段）

**范围**：仅自包含 mapping 字段 `dns`、`tun`、`hosts`、`profile`。B 类（proxies/proxy-groups/proxy-providers/rules/rule-providers）继续走现有专门合并逻辑。

**语义**（吸收 mihomo `mergeOverrideMaps` 递归）：

- mapping 字段：递归合并，叶子 primary 赢。
- sequence/标量：整块 primary 赢（与现状一致）。

```rust
fn deep_merge(base: &mut Mapping, supp: &Mapping, conflicts: &mut Vec<ConflictEntry>) {
    for (k, v) in supp {
        match (base.get(k), v) {
            (Some(Value::Mapping(b)), Value::Mapping(s)) => deep_merge(b, s, conflicts),
            (Some(_), _) => { /* 冲突：primary 赢，记 conflict */ }
            (None, _) => { base.insert(k.clone(), v.clone()); }
        }
    }
}
```

**注意**：B 类字段的关联性（group 引用 proxies 等）正是 deep merge 会破坏的，故 deep merge 严格限定在 A 类，避免「合并出引用不存在节点的 group」。

## 错误处理与 conflict 语义

统一通过 `ConflictEntry { field, name, source, reason }` 记录，存入 `IRuntime.merge_conflicts` 供前端展示。所有「冲突/降级」均不阻断配置生成；只有「读不到 profile 文件」等 I/O 错误才返回 Err。

## 测试策略

- 层 1：重名同定义 → 丢弃；重名不同定义 → 不丢（覆盖 proxies/providers/rule-providers）。
- 层 2：重名不同定义 → 重命名 + 引用重写正确（group `proxies`/`use`、rules `RULE-SET`）。
- 层 3：group 引用不存在的 proxy/provider/group → 记 conflict + 移除引用，不 panic。
- deep merge：dns 子字段并集；sequence 字段仍整块 primary 赢。
- 阶段 1：delete_item 清理 merged。
- 回归：现有 8 个测试全绿。

## 决策记录

- 2026-08-14：确认不抽库、不做阶段 4（重命名/过滤）、deep merge 收窄 A 类、三层引用完整性按推荐方案、层 2 重命名保底覆盖 proxies/proxy-providers/rule-providers 三类。
