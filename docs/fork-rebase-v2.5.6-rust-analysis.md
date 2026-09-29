# fork Rust 侧 vs 上游 v2.5.6：可替代 / 需重植 / 冲突面 结论

基线：fork `HEAD` = `c75c5b02`（tag `v2.5.2-fa.1010`），上游 `v2.5.6`，merge-base `28f2efc5`。
fork Rust 侧 delta 共 15 文件 `+1968/-22`，其中 fork-only 新文件 `enhance/multi_merge.rs`(1405) + `module/ssh_config.rs`(273) = 1678 行。
冲突面复核（`git merge-tree --write-tree HEAD v2.5.6` → tree `34ca43e2`）：Rust 侧 6 文件确与任务给定一致；`cmd/profile.rs`、`config/runtime.rs`、`core/manager/config.rs` **自动合并无冲突**。

> 定位：本文是 `.claude/skills/upstream-sync/SKILL.md` 第 2 步（复审上游 delta）与第 3 步（智能合并）的**决策依据**；该 skill 要求的"最终汇报只进对话、不落盘"仍然适用，本文不替代那 6 句汇报。分类法沿用 `.github/CONFLICT_RESOLUTION_GUIDE.md:174-182` 的 Type A–E。

---

## 1. 结论表

| # | fork 功能/补丁 | 上游 v2.5.6 是否已覆盖 | 处置建议 | 依据 |
|---|---|---|---|---|
| 1 | `enhance/mod.rs` 内联实现多订阅合并（`ProfileItems.current_uid`/`merge_conflicts`、`collect_profile_items` 改写、`enhance()` 4 元组），+104 行 | 功能否；但上游把 enhance 改成**纯函数** `enhance(&IProfiles)` + `collect_profile_items(&IProfiles)`，挂载面大改 | **重植，压到 1 行** | `v2.5.6:src-tauri/src/enhance/mod.rs:166`（`collect_profile_items(profiles)`）、`:173`（`.current_mapping()`）、`:877`（`enhance(profiles)`）；`HEAD:src-tauri/src/enhance/mod.rs:167-217,223-231,747-763` |
| 2 | 冲突日志走 `enhance()` 第 4 返回值 → `IRuntime.merge_conflicts` | **冲撞**：上游第 4 位是 `DnsOverrideState`；且上游已有同类副作用通道 `take_discarded_keys_notice()` | **删除**，改副作用通道 | `v2.5.6:enhance/mod.rs:877-880`（返回 `DnsOverrideState`）、`:411-441`（`DISCARDED_KEYS_NOTICE`/`notify_discarded_keys`/`take_discarded_keys_notice`）、`v2.5.6:cmd/runtime.rs:36`；`HEAD:config/runtime.rs:17`、`HEAD:config/config.rs:205,214`、`HEAD:core/manager/config.rs:30` |
| 3 | 把冲突塞进 `chain_logs["MultiMerge"]`（`ResultLog` 丢类型） | 否 | **删除**（同上通道） | `HEAD:enhance/mod.rs:846-858`；`v2.5.6:enhance/mod.rs:404-409`（`discarded_note` 是上游同类做法） |
| 4 | `ssh_config::inject_paths` 挂在 `enhance()` 末尾 | 否；但上游 `Config::generate_with_profiles()` 已有现成后处理段 | **重植：移到 `config/config.rs`** | `v2.5.6:config/config.rs:230-232`（`resolve_provider_path_conflicts` / `sanitize_tunnels_proxy` / `MixedPort`）；`HEAD:enhance/mod.rs:840`；`HEAD:module/ssh_config.rs:53` |
| 5 | `IProfiles::cleanup_orphaned_files` 里挂 SSH 孤儿清扫 | 上游**先补守卫后整体删除**该函数（#7577/#7585 → "preserve files until explicit deletion"） | **删除孤儿清扫**，只保留"显式删除才删文件" | `ef179fcf`（加守卫）、`44f6f6e1`（删调用点+函数）；`v2.5.6:config/profiles.rs` 无 `cleanup_orphaned_files`；`HEAD:config/profiles.rs:457,525-533`、`HEAD:config/config.rs:107` |
| 6 | 删除 profile 时 `remove_ssh_config(uid)` | 否；但上游新增 `ProfileDeletePlan`/`plan_delete_item` 是**现成挂载点** | **重植** | `v2.5.6:config/profiles.rs:61-88`（`ProfileDeletePlan::cleanup`）、`:335-372`（`plan_delete_item`）、`cmd/profile.rs:151,165`（`plan.cleanup().await`） |
| 7 | 把 `get_current`/`get_items` 补回并+`patch_merged` | 上游 `b7c4f7d3`(refactor: api cleanup) **已删两者**，v2.5.6 全部改直接字段访问（`.current` / `.items`） | **删除 `get_current`/`get_items` 补回**，只留 `patch_merged` | `b7c4f7d3`；`v2.5.6:config/profiles.rs` 无 `fn get_current`/`fn get_items`，且全仓无 `.get_current()` 调用；`HEAD:config/profiles.rs:139-152` |
| 8 | `PrfItem::from_url`/`from` 内做 SSH 抽取（`extract_and_sync`） | 否；上游两处都改过，其中 `normalize_profile_home_url` 是**安全修复** | **保留并重植**（严禁回退 `home` 行） | `v2.5.6:config/prfitem.rs:343-360`；`695fadf2`(fix(security): restrict external URL opening)；`HEAD:config/prfitem.rs:230,396` |
| 9 | `commit_merged_profiles` 用 `edit_draft`+`feat::enhance_profiles()`+`apply()/discard()` | 上游新增 `update_config_forced_with_profiles(candidate, rollback)`（校验+`save_file`+失败回滚，`delete_profile` 已用） | **重植：改用 `_with_profiles`**，并补 `CURRENT_SWITCHING_PROFILE`/`PROFILE_WRITE_LOCK` | `v2.5.6:core/manager/config.rs:187-231`、`:266-280`；`v2.5.6:cmd/profile.rs:144-175,317-336`；`HEAD:cmd/profile.rs:434-468` |
| 10 | `lib.rs` updater `default_version_comparator` 自定义比较 | **上游 `8982525f` 重建了同一槽位**（`is_build_to_stable`），语义不同但目标重叠 | **合并两者条件**（见 §5） | `8982525f`：`lib.rs:47-56` + `core/updater.rs:104-110,162`；`HEAD:lib.rs:49-70` |
| 11 | `update-viewer.tsx` 用 `openUrl()` + fork release URL | 上游 `695fadf2`（fix(security): restrict external URL opening）已删除 `openUrl`，改 `openExternalUrl` | **重植到 `openExternalUrl`**（安全相关） | `695fadf2`；`v2.5.6:src/utils/open-external-url.ts`；`v2.5.6:components/setting/mods/update-viewer.tsx:21,309,317-318` |
| 12 | `IProfiles.merged` 字段（profiles.yaml 持久化） | 上游 `IProfiles` 只有 `current` + `items`，**无 `deny_unknown_fields`** | **保留**，但须知道纯上游版会静默丢弃该键 | `v2.5.6:config/profiles.rs:48-53`；`HEAD:config/profiles.rs:57-58` |

---

## 2. 可替代清单（用什么替代、diff 减少多少）

1. **`enhance/mod.rs` 104 行 → 1 行。** 替代：在 `config/profiles.rs` 新增 `IProfiles::current_mapping_resolved()`（`merged` 非空时合并、否则回落 `current_mapping()`；沿用现有 `retain_*` 风格的"跳过读不到的条目"守卫），再把 `v2.5.6:enhance/mod.rs:173` 的 `.current_mapping()` 改成 `.current_mapping_resolved()`。**enhance/mod.rs 的 fork delta 归零**，`collect_profile_items`/`enhance()` 签名与返回元组 100% 取上游。
   - 不选"外层包一个合成 `IProfiles` 再调 `enhance(&synthetic)`"：`dns_override_source(profile_uid, …)`（`v2.5.6:config/dns.rs:24`）与 `verge.profile_dns_settings` 都按 uid 索引，合成 uid 会破坏 DNS 覆盖确认（`v2.5.6:feat/dns.rs:38,44`）。**已否定**。
2. **`IRuntime.merge_conflicts`（`config/runtime.rs:17`）+ `core/manager/config.rs:30` + `config/config.rs:205,214` 全部删除。** 替代：把冲突写进 `multi_merge.rs` 里的 `static` + `take_merge_conflicts()`（形态照抄上游 `v2.5.6:enhance/mod.rs:411-441`），`get_merge_conflicts` 从 `IRuntime` 改读该 static。省掉 3 文件改动 + `enhance()` 第 4 位的语义冲撞。
3. **`cleanup_orphaned`（`module/ssh_config.rs:214-237`）+ `profiles.rs` 孤儿钩子删除。** 后果：SSH 配置只在 `plan_delete_item` 路径删除。⚠️ 顺带修掉 fork 自带缺陷：fork HEAD 的调用点（`config/profiles.rs:525-533`）**无空列表守卫**，`items` 为空时会删光 `ssh-configs/`。见 §6-2。
4. **`get_current`/`get_items` 补回删除**（`config/profiles.rs:139-152` 中的后两个 fn）。上游已用 `.current`/`.items` 直接字段访问；`core/timer.rs:110,155,365` 的 `get_items()` 调用点上游也已改（`b7c4f7d3`）。
5. **`config/config.rs` 冲突消失。** 目前冲突来自 fork 改了 `generate()`，而上游把 `generate()` 拆成 `generate()` + `generate_with_profiles()`（`v2.5.6:config/config.rs:226-232`）。改完只在上游独有函数里加 2 行 → 不再冲突。
6. **`update-viewer.tsx`/`settings.tsx` 的 `openUrl` 补丁删除**，替换为 `openExternalUrl`（`695fadf2` 已限定仅 http/https，`v2.5.6:capabilities/migrated.json:77-86` 允许 `https://*`，fork 仓库 URL 不受限）。

**量化**：编辑类文件 fork delta 由 ≈`+290/-22` 降到 ≈`+160/-22`；Rust 冲突文件由 6 个降到 5 个（`config/config.rs` 出列），`enhance/mod.rs` 由 475 行整段冲突降到 1 行，`config/profiles.rs` 由 7 处降到 4 处。fork-only 新增文件（1678 行）不变，那是真正的功能资产。

---

## 3. 必须保留清单（重植落点）

1. **`enhance/multi_merge.rs` 全量保留（1405 行）** — 上游无等价能力。落点：零改动，仅在 `mod.rs` 加 `pub mod multi_merge;`。入口 `multi_profile_merge`（`:636`）、`ConflictEntry`（`:8`）、`DEEP_MERGE_FIELDS`（`:591`）。
2. **`IProfiles.merged` + `patch_merged`** — 落点 `v2.5.6:config/profiles.rs:49-53` 结构体内、`:149` `patch_config` 之后。注意 `patch_config` 只处理 `current`，不改。
3. **合并取值逻辑** — 落点在 `current_mapping()`（`v2.5.6:config/profiles.rs:376`）**旁边**新增方法，**不要改 `current_mapping()` 本体**（见 §6-1）。
4. **`reorder` 的 merged 同步重排** — 上游 `reorder` 已重写为 `(&str, &str)` + `rposition`（`v2.5.6:config/profiles.rs:217-231`）。落点：把 fork 的 merged 同步块（`HEAD:config/profiles.rs:234-250`）插进上游 `reorder` 的 `save_file()` 之前。
5. **删除时清理 `merged`** — 落点：`plan_delete_item`（`v2.5.6:config/profiles.rs:335-372`）内，用 `get_item(uid)?.option` 推出的 5 个关联 uid + 主体 uid 组 `HashSet`，`retain` 掉；`self.items = Some(items)` 之前。上游该函数已不在内部删文件（返回 `ProfileDeletePlan`），fork 的 uid 侧效应必须留在这里而不是 `cleanup()`。
6. **`remove_ssh_config` 的挂载点** — 二选一，推荐后者：
   - a) `v2.5.6:config/profiles.rs:66-88` `ProfileDeletePlan::cleanup()` 内（但它只拿到 `files: Vec<String>`，需要额外带 uid，得扩结构体字段）；
   - b) **`v2.5.6:cmd/profile.rs:165` `plan.cleanup().await;` 之后**加 `ssh_config::remove_ssh_config(&index)`。推荐 b：不动上游结构体，落点与 `PROFILE_WRITE_LOCK`/回滚语义同层。
7. **`ssh_config::extract_and_sync` 两处导入钩子** — 落点 `v2.5.6:config/prfitem.rs:360`（`from_url`，用 `&clash_data` 解析、`has_ssh` 参与校验）与 `from()` 的 `file_data` 分支。
8. **`inject_paths`** — 落点 `v2.5.6:config/config.rs:231` `generate_with_profiles`，在 `Config::runtime().await.edit_draft(...)` 之前对 `config` 调用，uid 取 `profiles.current`（`commit_merged_profiles` 已保证 = primary），并注意 `use_default_config`（`v2.5.6:core/manager/config.rs:132`）是另一条不经过 enhance 的路径。
9. **`set_merged_profiles`/`clear_merged_profiles`/`get_merge_conflicts` 三个命令** — 落点 `v2.5.6:lib.rs` invoke 列表（**上游在末尾新增了 `cmd::check_media_unlock_item`，并集保留**）；`commit_merged_profiles` 按 `delete_profile` 的形状重写（`CURRENT_SWITCHING_PROFILE` CAS → `PROFILE_WRITE_LOCK` → `update_config_forced_with_profiles(&candidate, &original)` → 失败 `handle_validation_notice`）。
10. **`lib.rs` updater 比较器** — 见 §5。
11. **`update-viewer.tsx` fork URL** — 落点 `v2.5.6:.../update-viewer.tsx:317-318`，用 `openUrlWithNotice(\`https://github.com/iuin8/clash-verge-rev/releases/tag/v${resolvedVersion}\`)`；`services/update.ts` 的 `resolveRemoteVersion` 上游仍存在但仍是**私有 `const`（`v2.5.6:src/services/update.ts:44`）** → 必须保留 fork 的 `export`。

---

## 4. 冲突点性质表

「类型」列用仓库既有分类法（`.claude/skills/upstream-sync/SKILL.md:71-80` 决策矩阵 + `.github/CONFLICT_RESOLUTION_GUIDE.md:178-182` Type A–E）：A=`// FORK:`/fork 意图，B=上游 bug fix/安全补丁，C=双方各自新增，D=依赖/API 清理。

| 文件 | 冲突位置(tree `34ca43e2`) | 性质 | 类型 | 合并策略 |
|---|---|---|---|---|
| `cmd/runtime.rs` | `:89-98` | 相邻新增（fork 插命令，上游删了一段空行） | A+C | 并集：保留 `get_merge_conflicts` |
| `config/config.rs` | `:225-232` | 同区域改写：fork 改 `generate()`，上游把 `generate()` 拆出 `generate_with_profiles()` | C | 取上游；fork 意图改装到 `generate_with_profiles` → **冲突可消除** |
| `config/prfitem.rs` | `:369-376` | 同行重叠（3 行内双方都动） | B+C | 并集：留 SSH 块，`from_str` 收 `&clash_data`，校验保留 `&& !has_ssh`；**`home` 行必须取上游**（`695fadf2` 安全修复） |
| `config/profiles.rs` | `:52-58` | 结构体相邻新增 | C | 并集 |
| | `:171-187` | 相邻新增（fork 加 `patch_merged`；上游删 `get_current`/`get_items`） | D+C | 取上游 + 只补 `patch_merged` |
| | `:254-278` | **同函数重写**（`reorder`：上游换 `&str`+`rposition`+块作用域） | A+C | 取上游，重植 merged 同步块 |
| | `:368-413` | **同函数重写**（`delete_item` → `plan_delete_item`，且不再内部删文件） | A+C | 取上游，重植 merged 清理 + `remove_ssh_config` |
| | `:479-650` | 上游**纯删除 171 行**（`cleanup_orphaned_files`/`REGEX_PROFILE_FILE`/`get_name_by_uid`） | **B**（`44f6f6e1` 最终取消孤儿清扫） | 全取上游；fork 的 SSH 孤儿钩子随之失效，按 §6-2 决策 |
| | `:685-706` | 相邻删除 + 签名收窄（`profiles_reorder_safe` 参数 `&String`→`&str`） | D | 取上游 |
| | `:1329-1467` | tests 相邻新增 | C | 并集（fork 的 2 个 `retain_merged_after_delete` 测试保留） |
| `enhance/mod.rs` | `:172-230`、`:235-250` | **同函数重写**（`collect_profile_items` 签名 + 全函数体） | A+C | 全取上游 + 1 行 |
| | `:953-973` | **同函数重写**（`enhance` 签名 + 返回元组） | A+C | 全取上游 |
| | `:1069-1543` | **475 行整段重写**（fork 的 `enhance()` 体 vs 上游新增 `AuthoritativeFields`/`process_global_items`/`process_profile_items`/2 个测试 mod） | A+C | 全取上游；按 §2.1 只动 1 行 |
| `lib.rs` | `:47-72` | 同槽位重写（两边都写 `default_version_comparator`） | A | 手工合并两个条件（§5），保留 fork updater 策略 |
| | `:237-243` | 相邻新增（fork 3 命令 vs 上游 `cmd::check_media_unlock_item`） | C | 并集 |

---

## 5. `lib.rs` updater 补丁裁决

- fork 补丁出自 `b9b6ba2f`（"修复 fork 客户端版本号比较 (A 方案)"，2026-05-17），条件是 **"major/minor/patch 相同 + 本地无 pre-release + 远端有 pre-release"**（`HEAD:lib.rs:49-69`）。它保护的场景是"客户端 version 停在无 `-fa.N` 的基线版本"。该场景已被同一次提交的第 1 项修复（`prepare-release.sh` 同步三个文件 version）消除：HEAD `package.json` = `2.5.2-fa.1010`、`src-tauri/Cargo.toml` = `2.5.2-fa.1010`、`tauri.conf.json` = `2.5.2-fa.1010`。所以该分支在正常发布下**不会触发**，剩下的是防御性兜底。
- 上游 `8982525f`(fix(updater): detect stable releases from build metadata versions) 在**完全相同的槽位**重写了比较器：`release.version > current || core::updater::is_build_to_stable(&current.to_string(), &release.version.to_string())`（`v2.5.6:lib.rs:47-56`，`v2.5.6:core/updater.rs:104-110`）。`is_build_to_stable` 只在"current 带 `+build` 元数据且 base == remote"时为真 —— **与 fork 条件方向相反**，无法替代 fork 的兜底；反过来 fork 的条件也无法替代 `+build` 场景。
- 另：`598b5e69`(feat(updater): sanitize release notes and standardize semver) **只改前端**（`package.json`/`update-viewer.tsx`/`services/update.ts`/`pnpm-lock`，0 行 Rust），不构成对 fork Rust 补丁的替代。
- **结论**：两个条件都要留，写成 `release.version > current || is_build_to_stable(...) || fork_cond(...)`。同时 `tauri-plugin-updater` 从 `2.10.1` 升到 `2.12.0`（`git diff v2.5.2..v2.5.6 -- src-tauri/Cargo.toml`），`default_version_comparator` 仍存在（上游自己在用），**未验证** 2.12.0 是否改变 `current: &Version` / `release.version` 的类型或 `pre` 的可访问性 —— 编译时确认。

---

## 6. NEEDS_USER_REVIEW

1. **合并是否要走"共享的 `current_mapping()`"。** `dns_override_source`（`v2.5.6:config/dns.rs:24-48`）对当前 profile 的 `dns` 块做 SHA256 指纹，用于"确认过的 DNS 覆盖不重复弹窗"；而合并在 `DEEP_MERGE_FIELDS`（`multi_merge.rs:591`）里**深合并 `dns`**。若把合并塞进 `current_mapping()`（`v2.5.6:feat/dns.rs:38` 与 enhance 共用），指纹会随补充订阅变化 → 每次改多订阅都会重新弹"确认 DNS 覆盖"。本报告建议**新增独立方法**以规避，但"合并后的 dns 才是真实来源"这一语义取向需要人拍板。
2. **是否彻底放弃 SSH 孤儿清扫（属 Type B：上游 bug fix 触到 fork 代码）。** 准确时间线（易误判，务必按此）：
   - `v2.5.2`（fork 基线）：`cleanup_orphaned_files`（`v2.5.2:config/profiles.rs:420`）**没有**空列表守卫；fork 的 SSH `cleanup_orphaned` 就挂在这个无守卫的函数末尾（`HEAD:config/profiles.rs:525-533`）。**即 fork HEAD 当前自己就带着 #7577 类误删风险**（`items` 为空时 `valid_uids` 为空 → 删光 `ssh-configs/*.conf`）。
   - `ef179fcf`(2026-07-31, #7585)：拆出 `cleanup_orphaned_files_in` 并**新增**空列表守卫。
   - `44f6f6e1`(2026-08-30, "preserve files until explicit deletion")：**同时删除**调用点（`config/config.rs:135-138`）和整个函数（含守卫）。
   - 因此上游的最终选择是"**取消全局清扫**"而非"加守卫"。skill 决策矩阵（`SKILL.md:75`）要求吸收该修复原理 → **推荐直接放弃 SSH 孤儿清扫**，只保留 `plan_delete_item` 路径的删除（§3-6）。若因敏感残留必须保留清扫，则必须自带"`items` 为空则跳过"守卫**并**限定只扫 `ssh-configs/*.conf`，不得沿用上游已删除的 `get_all_active_files` 口径。
3. **`merged` 的持久化位置。** 现在写在 `profiles.yaml`。上游 `IProfiles` 无 `deny_unknown_fields`，退回纯上游版会**静默丢弃**该键（并且下一次 `save_file()` 把它从文件抹掉），用户多订阅选择会无声丢失。是否改为独立文件（如 `merged.json`）承载，需决策。
4. **updater 条件 4 的语义可接受性。** fork 条件会判定"稳定版 → 预发布版"是升级，对上游语义是反的。若某次误把 `2.5.6`（无后缀）当 current，则 `2.5.6-fa.1` 会被判为可升级。要确认这是有意的兜底而非隐患。
5. **`DEEP_MERGE_FIELDS` 里 `"profile"` 与 `Merge` 扩展的叠加顺序（已核实，但需人确认取舍）。** 管线顺序是：多订阅合并（fork，产出 `config`）→ … → `process_global_items` → `process_profile_items`（`v2.5.6:enhance/mod.rs:940-958`），即 **`Merge` 扩展在 multi-merge 之后执行**。而 `Merge` 扩展的默认模板 `ITEM_MERGE` 自带 `profile: {store-selected: true}`（`v2.5.6:utils/tmpl.rs:10-14`，`v2.5.6:config/prfitem.rs:417` `from_merge`），`use_merge` 对 `profile` 走 `deep_merge`（`v2.5.6:enhance/merge.rs:19-31`，仅 `dns`/`hosts` 特判）。结论：**补充订阅提供的 `profile.store-selected` 恒被 GUI 覆盖**（属预期，GUI 应拥有该键），但 `profile` 下其他子键（如 `store-fake-ip`）会存活。需确认这是期望语义，还是应把 `"profile"` 从 `DEEP_MERGE_FIELDS`（`multi_merge.rs:591`）移除。**未验证**：`Config::clash()`（IClashTemp）是否也含 `profile` 键并因此构成第三条覆盖路径。
   - 附：我最初设想的"上游 `utils/resolve/*` / `config/snapshot.rs` 新增了对 `profile` 段的依赖"这一说法，**经核查为误判**——`snapshot.rs` 是 `FileSnapshot`（文件字节快照，`v2.5.6:config/snapshot.rs:1-30`），与 YAML `profile` 键无关；全仓除测试字符串外无对顶层 `"profile"` 键的引用。已撤回。
