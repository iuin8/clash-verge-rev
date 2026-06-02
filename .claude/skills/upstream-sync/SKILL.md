---
name: upstream-sync
description: 'Use when 用户要求同步上游 clash-verge-rev/clash-verge-rev、合并 upstream、merge upstream、sync upstream、升级 fork 到上游新 release tag (v*.*.*) / fa/<tag>-fa.0 分支；或处理 ours/theirs 冲突决策、// FORK 标记冲突 conflict resolution、同步期间的脚本退出码 2 / 3。不用于发布 fork 版本（那是 release / prerelease 技能）。'
---

# Upstream Sync — clash-verge-rev fork 升级流程

## 核心原则

> **冲突文件不能无脑取 ours，也不能无脑取 theirs。**
> 本 fork 走「`// FORK:` 标记 + 智能合并」哲学：每个冲突文件都按
> "上游 delta 复审 → 决策 → 应用 → 汇报" 四步处理。绝大多数冲突（CI / 版本 / 功能集成）
> 都要"保留 fork 块 + 吸收上游改动"双向融合，而非单边取舍。
> 哪怕最终决定仍 `--ours`，理由必须出现在最后的汇报中，给用户留复盘依据。

> 详细冲突规则（Type A–E、特殊场景、提问模板）见 `.github/CONFLICT_RESOLUTION_GUIDE.md`，
> 本技能不重复，只补充本地脚本流程与决策矩阵。CI 自动 PR 流程见 `.github/UPSTREAM_SYNC_GUIDE.md`
> —— 本技能是**本地手动 / AI 驱动**的同步路径，与 CI 的 `upstream-sync.yml` 互补。

## 执行流程

```
脚本（机械操作）─→ 退出码 0：进入 Step 2 复审
                  退出码 2：→ 进入 Step 2 复审 + Step 3 智能合并
                  退出码 3：验证失败 → 进入 Step 4 验证修复
                  退出码 1：环境错误 / 已是最新 → 读 stderr 处理
```

### 本技能 vs CI 工作流（分支命名互补）

二者分支命名空间不同，可并存，不冲突：

| 维度 | 本技能（本地手动）                              | CI（`.github/workflows/upstream-sync.yml`） |
| ---- | ----------------------------------------------- | ------------------------------------------- |
| 分支 | `fa/<TARGET_TAG>-fa.0`（与 fork 发布分支一致）  | `upstream-sync/<TARGET_TAG>`                |
| 触发 | 用户手工跑脚本                                  | 每日定时 / `workflow_dispatch`              |
| 用途 | 已知新版本，需深度合并决策 + 本地编译验证后再推 | 定时巡检上游、无冲突时自动开 PR             |
| 冲突 | 本技能 Step 2–3 智能合并                        | 开 Draft PR、@claude 协助                   |

何时用本技能：需立即同步、想本地审完编译过再推、或接手 CI PR 遗留的冲突（本地按 Step 3 解完回推）。

---

## Step 1：运行脚本

```bash
./.claude/skills/upstream-sync/upstream-sync.sh
# 指定版本: ... v2.4.8     仅预检: ... --check-only     跳过编译验证: ... --no-verify
```

- **退出码 0**：无残余冲突且快速验证通过，但仍需 Step 2 复审 + Step 4 完整验证。
- **退出码 2**：输出 `SMART_MERGE_REQUIRED` + 冲突文件列表 + `AI_HINTS`，进入 Step 2 + Step 3。
- **退出码 3**：合并完成但 typecheck / cargo check 失败，进入 Step 4。

脚本只对 `Changelog.md` 自动 `--ours`（纯 fork 发布说明，会被 `prepare-release.sh` 重生成）。
**其余一切冲突都留给本技能智能合并** —— 这是与 mihomo fork 的关键差异（那边 CI 文件是纯 `--ours`，
这边 CI 文件需要吸收上游结构修复）。

---

## Step 2：复审（每次必做）

### 2-A. 基线已由脚本推导

脚本第 [0/5] 步自动从 fork 版本字段推导上次同步基线（与 mihomo 用 commit-message grep 不同 ——
本 fork 没有 "merge upstream" commit 规范，但每次发布都会更新版本字段，它是零维护成本的可靠来源）：

```bash
FORK_VERSION="$(jq -r '.version' package.json)"   # 例: 2.4.7-fa.1043
PREV_TAG="v${FORK_VERSION%%-fa.*}"                # 去 -fa.N → v2.4.7
```

`AI_HINTS` 里的 `PREV_TAG` / `TARGET_TAG` 即此结果。直接用 `PREV_TAG..TARGET_TAG` 做 delta 审查。

### 2-B. 看上游 delta

```bash
for f in <AI_HINTS 列出的 GIT_OVERLAP_FILES 与冲突文件>; do
  echo "=== $f ==="
  git log --oneline "$PREV_TAG".."$TARGET_TAG" -- "$f"   # 上游本次提交摘要
  git diff "$PREV_TAG".."$TARGET_TAG" -- "$f"            # 上游具体改动
done
```

### 2-C. 应用决策矩阵（对齐 CONFLICT_RESOLUTION_GUIDE 的 Type A–E）

| 冲突内容                           | 决策                   | 操作                                                                    |
| ---------------------------------- | ---------------------- | ----------------------------------------------------------------------- |
| 含 `// FORK:` 块 vs 上游改动       | **保留 fork 块**       | 上游若仅在 fork 块外改动，并存；上游重构同一段 → 在新结构上重植 fork 块 |
| 上游 bug 修复 / 安全补丁 / CVE     | **必须吸收**           | 与 fork 块融合；若改到同一逻辑，把修复原理应用到 fork 代码              |
| 上游新功能（不碰 fork 区）         | **接受并存**           | 保留双方                                                                |
| 上游依赖升级（deps/Cargo）         | **接受 theirs**        | 用上游新版本号，合并后验证 fork 代码兼容                                |
| 版本 / 签名（见下方"版本文件"）    | fork 方案              | 保留 fork `version`/`pubkey`/`endpoints`，接受上游 deps / 新配置项      |
| fork 故意禁用的功能（`if: false`） | **保留 `--ours` 意图** | 但吸收上游对该 job 的结构更新（如新 `needs:`）                          |
| 模糊地带                           | 标 `NEEDS_USER_REVIEW` | 暂保留 `--ours`，汇报中列出供用户拍板                                   |

### 2-D. 复审"git 自动合并成功"的文件（AI_HINTS → GIT_OVERLAP_FILES）

> git 三向合并成功 ≠ 语义正确，更 ≠ fork **独有文件**仍能编译。

对每个 overlap 文件跑 2-B 的 delta 审查，并检查对 **fork 独有文件 / 无标记扩展** 的连带影响：

- 上游改了 `enhance/mod.rs` 的类型 / 函数签名？→ `enhance/multi_merge.rs` 还兼容吗？
- 上游改了 `profile-item`/`profile-box` 的 props？→ `conflict-viewer.tsx` / `merge-order-bar.tsx` 还能编译吗？
- 上游改了 IPC 命令签名（`invoke()`）/ `IRuntime` / `IProfileItem` 等共享结构？→ **高风险无标记点**：
  fork 在 `src/services/cmds.ts`（末尾新增 `setMergedProfiles`/`clearMergedProfiles`/`getMergeConflicts` 包装）
  与 `src/types/global.d.ts`（新增 `ConflictEntry` 等类型）的扩展**无 `// FORK:` 标记**、脚本检不出，
  必须人工核对其类型声明是否仍对得上 Rust 端签名。

发现连带影响时，把受波及文件同步改造，作为合并完整性的一部分（Step 4 编译期是最后兜底）。

---

## Step 3：智能合并（脚本退出码 2 时）

### 3-A. 读取冲突上下文

```bash
cat <conflict-file>                                      # 含冲突标记的完整文件
git diff "$PREV_TAG".."$TARGET_TAG" -- <conflict-file>   # 上游改了什么
git diff "$PREV_TAG"...HEAD       -- <conflict-file>     # fork 相对基线改了什么（理解 fork 意图）
grep -n "FORK:" <conflict-file>                          # 定位带标记的 fork 块
# 大文件（如 enhance/mod.rs 700+ 行 / profiles.tsx）补扫无标记的 fork 扩展（变量引用、跨文件调用）：
grep -En "FORK:|multi_merge|merged|merge_conflicts|selectedProfiles" <conflict-file>
```

### 3-B. 按文件类合并

**① 功能集成文件**（`AI_HINTS → FORK_MARKER_FILES`，如 `enhance/mod.rs`、`cmd/profile.rs`、
`config/profiles.rs`、`lib.rs`、`pages/profiles.tsx`、`profile-item.tsx`）：

fork 在这些上游文件里植入了「多订阅合并 (multi-profile merge)」功能。识别原则：

- `<<<<<<< HEAD` 块中带 `// FORK:` 注释 / 引用 `multi_merge` / `merged` / `conflict` / `selectedProfiles`
  / `merge_conflicts` 的是 fork 扩展 → **保留**
- `>>>>>>> <TAG>` 块是上游改动 → **吸收**
- 上游重构同一函数 → 在上游新结构上重新植入 fork 扩展，保证 fork 功能不丢、可编译

> ⚠️ **`FORK_MARKER_FILES` 只覆盖第①类，fork 改动其实有三类**，后两类脚本 `grep "FORK:"` 检不出：
>
> 1. **带 `// FORK:` 标记的块**（脚本可检出 → 在 `FORK_MARKER_FILES`）
> 2. **fork URL 替换、无标记**（`pages/settings.tsx`、`components/setting/mods/update-viewer.tsx`：
>    把 GitHub 链接从 `clash-verge-rev/clash-verge-rev` 改成 `iuin8/clash-verge-rev`）
> 3. **末尾新增 fork 函数、无标记**（`services/cmds.ts` 的 `setMergedProfiles` 等、`types/global.d.ts` 的新类型）
>
> 对**所有** fork 改过但不在 `FORK_MARKER_FILES` 的文件，必须人工补扫确保 fork 改动被保留：
>
> ```bash
> git diff "$PREV_TAG"...HEAD -- <file>   # fork 相对基线的改动（无标记的 URL/新增函数都在这里）
> ```
>
> 常见无标记改动：仓库 owner URL 替换（→ `iuin8`）、追加到文件尾部的 IPC 包装 / UI hook。

**② 版本 / 签名文件**（`AI_HINTS → VERSION_FILES`，`package.json` / `Cargo.toml` / `tauri.conf.json`）：

- `version`：用 fork 方案 `<TARGET 去 v>-fa.0`（如 `2.4.8-fa.0`）；后续 `-fa.NNNN` 递增交 `release` 技能
- `tauri.conf.json` 的 updater `pubkey` 与 `endpoints`：**必须保留 fork 值**（fork 自有 minisign 签名密钥
  - fork 仓库 URL，取上游会断更新签名链）
- 依赖 / 新配置项：**接受上游**，合并后 `pnpm install` / `cargo check` 验证兼容

**③ CI 文件**（`release.yml` / `updater.yml` / `scripts/*.mjs`）：

- 保留所有 `# FORK: disabled` / `if: false`（ARM Linux、fixed WebView2、winget、Telegram 等禁用 job）
- 保留 fork 仓库 URL / 精简逻辑
- 吸收上游对**启用中** job 的结构修复（新 step、新 `needs:`、action 版本升级）
- ⚠️ **禁用 job 的连带依赖**：禁用 job 虽保留定义，但上游可能改了**启用 job 对它的引用**。
  例：上游把 `update_tag` 的 `needs:` 从 `[release, release-for-linux-arm]` 改成 `[release]` ——
  必须**吸收**这个删除，否则 `update_tag` 会一直等一个 `if:false` 永不产出的 job，CI 卡死。
  逐个核对启用 job 的 `needs:` 是否引用了 fork 禁用的 job。

**④ i18n 源文件**（`AI_HINTS → I18N_SOURCE_FILES`，`src/locales/*/profiles.json`，13 个语言）：

- **并集合并**：fork 新增的 multi-merge UI key + 上游对已有 key 的改动，二者都保留
- JSON 无注释，无 `// FORK:` 标记，靠 key 名判断（fork 新增 key 见 `git diff "$PREV_TAG"...HEAD`）

### 3-C. 写入合并结果

用 Edit 去除所有 `<<<<<<<`/`=======`/`>>>>>>>` 标记。

### 3-D. i18n 生成文件 —— 不可手改（`AI_HINTS → REGEN_I18N`）

`src/types/generated/i18n-keys.ts`、`i18n-resources.ts` 是 `pnpm i18n:types` 的产物。
**先合好 ④ 的 locales 源文件**，再重新生成（仅运行，git add 留到 3-E 统一做）：

```bash
pnpm i18n:types          # = node scripts/generate-i18n-keys.mjs，覆盖 generated 文件
```

### 3-E. 统一标记已解决（原子化 git add）

把 3-C 的冲突消解结果（功能集成 / 版本 / locales 源）与 3-D 重新生成的文件**一次性** add，避免漏 stage：

```bash
git add \
  <3-C 已消解的功能集成 / 版本文件> \
  src/locales/*/profiles.json \
  src/types/generated/i18n-keys.ts src/types/generated/i18n-resources.ts
git status   # 确认无残留 UU / AA 未合并标记
```

---

## Step 4：验证（每次必须执行）

```bash
# 0. 若改过 locales：先重新生成 i18n 类型（见 3-D）
pnpm i18n:types

# 0.5 fork 配置完整性自检（编译过不了关，但取错 = 静默事故）
#     updater 签名链：pubkey + endpoints 必须是 fork 的，否则旧客户端永远收不到更新
grep -q 'iuin8/clash-verge-rev' src-tauri/tauri.conf.json \
  || echo '❌ tauri.conf.json updater.endpoints 丢了 fork URL（iuin8），更新签名链会断！'
git diff "$TARGET_TAG" HEAD -- src-tauri/tauri.conf.json | grep -E 'pubkey|endpoints' || true   # 确认指向 fork
# 版本字段：应是 fork 方案而非上游裸版本
jq -r '.version' package.json   # 期望 <TARGET 去 v>-fa.N（如 2.5.1-fa.0）

# 1. 依赖（上游可能升级了 deps）
pnpm install

# 2. 前端
pnpm typecheck          # tsc --noEmit
pnpm lint               # eslint --max-warnings=0（零警告强制）

# 3. Rust
cargo clippy-all        # 别名 = clippy --all-targets --all-features -- -D warnings
cargo fmt -- --check
cargo test

# 4. 重型最终验证（可选，耗时数分钟，需 pnpm prebuild 下好 sidecar）
pnpm build
```

失败时常见原因：

1. **上游改了类型 / 函数签名** → fork 独有文件（`multi_merge.rs` / `conflict-viewer.tsx`）或功能集成处编译失败 → 同步改造
2. **上游升级 / 新增依赖** → `git diff "$TARGET_TAG" HEAD -- package.json Cargo.toml`，跑 `pnpm install` / `cargo check`
3. **i18n key 不一致** → 漏跑 `pnpm i18n:types`，或 locales 并集没合全 → 补齐后重新生成

判断失败是上游破坏性变更还是合并引入，针对性修复。

---

## Step 5：提交与汇报

### 5-A. 提交

合并 commit 已由脚本创建（退出码 0 时）。Step 2–4 的修复作为独立 follow-up commits：

- `fix(<module>): adapt to upstream <change>` — 签名 / 类型 / 依赖同步
- `fix(i18n): regenerate types after locale merge`
- 若脚本退出码 2，由你完成所有 `git add` 后提交：

  ```bash
  git commit -m "chore: merge upstream $TARGET_TAG

  - 保留 fork multi-profile-merge 集成 (<files>)
  - 保留 fork 版本/pubkey/endpoints 与 CI 禁用 job
  - 吸收上游 <bug fix / deps / 新功能>"
  ```

### 5-B. 推送

```bash
git log --oneline -8
git push -u origin fa/<TARGET_TAG>-fa.0
```

> 同步完成后如需发布 fork 版本（`v<TARGET>-fa.NNNN`），交给 `release` 技能（它会读 `package.json`
> 基线、跑 `prepare-release.sh`、监控构建）。本技能止于"分支可编译、可推送"。

### 5-C. 汇报骨架（写在主对话**最后一条消息**，不落盘）

按这 6 个 bullet 组织，每条一句话；`AI_HINTS` 块就是 1–4 的输入：

1. **脚本自动 --ours 文件**（`SCRIPT_AUTO_RESOLVED`）：复审结论（通常 Changelog 无需吸收上游）
2. **版本 / 签名 / CI 文件**（`VERSION_FILES` + CI 类）：保留了哪些 fork 值、吸收了哪些上游改动
3. **功能集成智能合并**（`FORK_MARKER_FILES`）：上游意图 / fork multi-merge 意图 / 融合策略
4. **连带影响 + i18n**（`GIT_OVERLAP_FILES` / `I18N_SOURCE_FILES` / `REGEN_I18N`）：fork 独有 / 无标记文件是否被波及、locales 是否并集合并并重新生成 i18n 类型
5. **验证**：typecheck / lint / clippy / test / （build）/ 推送 short SHA
6. **NEEDS_USER_REVIEW** + **异步事项**：拿不准的项问用户拍板；可选清理不阻塞当前任务

---

## Fork 改动范围说明

该 fork 在上游基础上的核心功能：**多订阅合并 (multi-profile merge)** —— 同时激活多个订阅，
按拖拽顺序合并 YAML，UI 展示合并冲突。spec 见 `docs/superpowers/specs/2026-03-28-multi-profile-merge-design.md`。

**Fork 新增文件**（上游无，不会冲突，但可能因上游改动间接受影响）：

```
src-tauri/src/enhance/multi_merge.rs          # 合并引擎（核心）
src/components/profile/conflict-viewer.tsx    # 冲突查看器 UI
src/components/profile/merge-order-bar.tsx    # 合并顺序条 UI
scripts/prepare-release.sh                    # fork 发布脚本
.github/workflows/upstream-sync.yml           # CI 自动同步（与本技能互补）
```

**Fork 修改的上游共享文件**（冲突高发区，按类处理见 Step 3-B）：

> 表中 `{a,b}` 是 brace 展开简写（如 `profile-{box,item}.tsx` = `profile-box.tsx` + `profile-item.tsx`）。

| 类别                                   | 文件                                                                                                                                                                                      | 默认策略                                                  | FORK 标记                             |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------- | ------------------------------------- |
| 功能集成-Rust                          | `src-tauri/src/enhance/mod.rs` `src-tauri/src/cmd/{profile,runtime}.rs` `src-tauri/src/config/{config,profiles,runtime}.rs` `src-tauri/src/core/manager/config.rs` `src-tauri/src/lib.rs` | 智能合并                                                  | 有 `// FORK:`                         |
| 功能集成-TS（有标记）                  | `src/pages/profiles.tsx` `src/components/profile/profile-{box,item}.tsx`                                                                                                                  | 智能合并                                                  | 有 `// FORK:`                         |
| fork 扩展-TS（**无标记**，脚本检不出） | `src/services/cmds.ts`（末尾新增 IPC 包装）`src/types/global.d.ts`（新类型）`src/pages/settings.tsx`（fork URL）`src/components/setting/mods/update-viewer.tsx`（fork URL）               | 智能合并（保留 fork 新增 / URL，吸收上游其余）            | **无**，靠 `git diff PREV..HEAD` 补扫 |
| 版本 / 签名                            | `package.json` `src-tauri/Cargo.toml` `src-tauri/tauri.conf.json`                                                                                                                         | 智能合并（保 version/pubkey/endpoints，接受上游 deps）    | 部分                                  |
| CI / 发布                              | `.github/workflows/release.yml` `.github/workflows/updater.yml` `scripts/{updater,updater-fixed-webview2,release-version,prebuild}.mjs` `.gitignore`                                      | 智能合并（保留 `if:false` + 吸收上游结构修复 / `needs:`） | 有 `# FORK:`                          |
| i18n 源                                | `src/locales/*/profiles.json` (×13)                                                                                                                                                       | 并集合并                                                  | 无                                    |
| i18n 生成                              | `src/types/generated/i18n-{keys,resources}.ts`                                                                                                                                            | `pnpm i18n:types` 重新生成                                | 无                                    |
| 发布说明                               | `Changelog.md`                                                                                                                                                                            | `--ours`（脚本自动）                                      | 无                                    |

---

## 快速参考：冲突文件决策

| 冲突文件                                                                                    | 默认策略                                           | 复审强度                                                |
| ------------------------------------------------------------------------------------------- | -------------------------------------------------- | ------------------------------------------------------- |
| `Changelog.md`                                                                              | `--ours`（脚本自动）                               | 低（会被 prepare-release 重生成）                       |
| `src/types/generated/*`                                                                     | 重新生成（`pnpm i18n:types`）                      | **不可手改**                                            |
| `src/locales/*/profiles.json`                                                               | 并集合并                                           | 中（漏 key 会导致 i18n 生成不全）                       |
| 含 `// FORK:` 的功能 / CI / 版本文件                                                        | 智能合并                                           | **高** Step 3-B                                         |
| **无标记 fork 改动**（`services/cmds.ts`/`global.d.ts`/`settings.tsx`/`update-viewer.tsx`） | 智能合并（保留 fork 新增 / URL）                   | **高**（脚本检不出，靠 `git diff PREV..HEAD` 人工补扫） |
| `package.json`/`Cargo.toml`/`tauri.conf.json`                                               | 智能合并（保 version/pubkey/endpoints，接受 deps） | **高**                                                  |
| Fork 新增文件（`multi_merge.rs`/`conflict-viewer.tsx`/…）                                   | 无冲突                                             | Step 2-D 查间接影响 + Step 4 编译兜底                   |
| 其他上游文件（fork 未改过）                                                                 | `--theirs`（git 自动）                             | Step 4 编译 / 测试兜底                                  |
| **git 自动合并成功但双方都改过的文件**                                                      | （无冲突标记）                                     | **必做** Step 2-D 连带影响审查                          |

---

## Red Flags — 看到这些念头立即停下重走流程

| 错误念头                                         | 真相                                                                                                                                                     |
| ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| "脚本退出码 0 = 干完了"                          | 错。Step 2 复审 + Step 4 完整验证（lint/clippy/test）是脚本之后才发生的事                                                                                |
| "冲突文件一律 --ours 保住 fork 就行"             | 错。本 fork 是智能合并哲学：CI / 版本 / 功能文件都要**同时**吸收上游 bug 修复与依赖升级                                                                  |
| "git 没报冲突 = 文件没问题"                      | 错。上游可能改类型 / 签名，fork 独有文件（`multi_merge.rs` / `conflict-viewer.tsx`）编译期才炸                                                           |
| "generated i18n 文件手改一下就行"                | 错。它是 `pnpm i18n:types` 产物 —— 先合 locales 源、再重新生成，手改必漂移                                                                               |
| "tauri.conf.json 直接取上游"                     | 错。会丢 fork 的 updater `pubkey`（自有签名密钥）和 `endpoints`（fork 仓库 URL），更新签名链断裂                                                         |
| "typecheck + clippy 过 = 合并完成"               | 错。还要 `cargo test`、`pnpm lint`（零警告）、i18n 一致性；功能契合度（multi-merge 仍工作）也要审                                                        |
| "汇报写到 docs 里"                               | 错。Step 5-C 汇报只进对话最后一条消息，不落盘（避免文档膨胀 + git 噪声）                                                                                 |
| "同步完顺手把版本号 bump 成 -fa.NNNN 发布了"     | 错。本技能止于可推送分支；发布走 `release` 技能                                                                                                          |
| "只要冲突文件里有 FORK 标记我就找全了 fork 改动" | 错。fork 还有**无标记**改动（`cmds.ts` 末尾新增函数、`settings.tsx`/`update-viewer.tsx` 的 URL 替换），脚本检不出，必须 `git diff PREV..HEAD` 逐文件补扫 |
| "脚本退出码 3 = 小问题，先推了再修"              | 错。typecheck / cargo check 失败说明合并已破坏类型或结构，必须在 Step 4 完整验证前修完；推上去 CI 一样挂                                                 |
| "`--no-verify` 跳过了验证，那 Step 4 也能省"     | 错。`--no-verify` 只跳过脚本的**快速**门禁，Step 4 的完整 lint/clippy/test/i18n 校验是强制的最后防线，不可选                                             |
