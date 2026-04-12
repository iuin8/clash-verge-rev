---
name: release
description: >-
  发布 clash-verge-rev 新版本 - 自动获取当前版本，建议新版本号，
  询问用户确认，运行发布脚本更新 Changelog.md，创建 tag 并推送，
  监控 GitHub Actions 构建（每分钟轮询，超时30分钟），
  构建失败时自动诊断修复并重试，最多循环3次直到 Release 正常。
  TRIGGER when: 用户说"发布"、"release"、"新版本"、"打tag"或请求发布操作。
  DO NOT TRIGGER when: 用户只是询问版本信息或查看 Changelog。
---

# Release — 发布新版本

自动化 clash-verge-rev 的完整发布流程，从版本号确认到 GitHub Release 验收。

## 整体流程

```
发布准备 → 推送 tag → [监控循环] → 验收 Release
                          ↓ 失败
                       诊断修复 → 新 tag → 重试（最多3次）
```

---

## 执行步骤

### 1. 获取当前版本信息

```bash
git fetch --tags
LATEST_TAG=$(git tag --sort=-version:refname | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9]+\.[0-9]+)?$' | head -1)
UPSTREAM_VERSION=$(jq -r '.version' package.json)
```

### 2. 建议版本号

根据最新 tag 自动递增（increment 为4位数）：

- 最新 `v2.4.7-fa.1030` → 建议 `v2.4.7-fa.1031`
- 上游升级到 2.4.8 → 建议 `v2.4.8-fa.1001`

版本号格式：`v{upstream_version}-fa.{increment}`

### 3. 询问用户确认

使用 AskUserQuestion 工具询问：

```
当前最新版本: v2.4.7-fa.1030
上游版本: v2.4.7
建议新版本: v2.4.7-fa.1031

请选择：
1. 使用建议版本 v2.4.7-fa.1031
2. 自定义版本号
```

### 4. 执行发布脚本

```bash
./scripts/prepare-release.sh v2.4.7-fa.1031
```

脚本会：

- 分析自上次 tag 以来的所有 commit messages
- 自动分类（修复/新增/优化）生成 Changelog.md 条目
- 显示内容供用户确认
- 提交 Changelog.md、创建 tag、推送到远程

### 5. 监控构建（每分钟轮询，超时30分钟）

推送成功后立即进入监控循环，使用 ScheduleWakeup 每 60 秒唤醒一次：

```bash
# 获取本次 tag 触发的 workflow run
gh run list \
  --workflow=release.yml \
  --limit=5 \
  --json databaseId,status,conclusion,headBranch,event \
  --jq '.[] | select(.event == "push")'

# 查看指定 run 状态
gh run view <RUN_ID> --json status,conclusion,jobs \
  --jq '{status,conclusion,jobs:[.jobs[]|{name,status,conclusion}]}'
```

**状态判断：**

| status                   | conclusion              | 动作             |
| ------------------------ | ----------------------- | ---------------- |
| `in_progress` / `queued` | —                       | 等待，60秒后再查 |
| `completed`              | `success`               | 进入验收步骤     |
| `completed`              | `failure` / `cancelled` | 进入诊断修复     |
| 超过30分钟仍未完成       | —                       | 超时，通知用户   |

监控时向用户实时报告进度，例如：

```
[第3分钟] 构建进行中 — check_tag_version ✅  release(windows) 🔄  release(macos) 🔄  release(linux) 🔄
[第15分钟] 构建进行中 — release ✅  update_tag 🔄
[第22分钟] 构建完成 ✅
```

### 6. 验收 Release

构建成功后，验证 Release 页面内容：

```bash
# 检查 release 是否存在且已发布（非 draft）
gh release view <TAG> --json isDraft,body,assets \
  --jq '{isDraft, hasBody: (.body | length > 50), assetCount: (.assets | length)}'
```

**验收标准（全部满足才算通过）：**

1. `isDraft: false` — 已发布，非草稿
2. `hasBody: true` — release body 超过50字符（有实质内容）
3. release body 包含当前版本号（如 `v2.4.7-fa.1031`）
4. `assetCount >= 3` — 至少有 Windows/macOS/Linux 三个平台的产物

验收通过后告知用户：

```
✅ Release v2.4.7-fa.1031 发布成功
   - 平台产物: 5 个文件
   - Changelog: 已包含
   - https://github.com/iuin8/clash-verge-rev/releases/tag/v2.4.7-fa.1031
```

---

## 自动修复循环（最多3次）

构建失败或验收不通过时，进入修复循环。**每次循环版本号自动递增**（如 1031 失败 → 修复后发 1032）。

### 失败类型诊断

```bash
# 查看失败 job 的日志
gh run view <RUN_ID> --log-failed
```

**根据失败 job 定位问题：**

| 失败 job            | 可能原因                         | 修复方向                          |
| ------------------- | -------------------------------- | --------------------------------- |
| `check_tag_version` | Changelog.md 缺少当前版本条目    | 运行 `prepare-release.sh`         |
| `check_tag_version` | package.json 版本与 tag 不符     | 更新 package.json 版本            |
| `release`（构建）   | Rust/TS 编译错误                 | 修复代码错误后重新发布            |
| `update_tag`        | softprops/action-gh-release 失败 | 检查权限，或手动更新 release body |
| `release-update`    | updater release 不存在           | 检查 `updater` tag 是否存在       |

### 修复后重新发布

修复完成后，递增版本号重新走完整流程：

```bash
# 新版本 = 上一次失败版本 + 1
./scripts/prepare-release.sh v2.4.7-fa.1032
```

**循环计数管理：**

```
第1次尝试: v2.4.7-fa.1031 → 失败 → 修复
第2次尝试: v2.4.7-fa.1032 → 失败 → 修复
第3次尝试: v2.4.7-fa.1033 → 失败 → 停止，通知用户
```

第3次仍失败时，输出完整诊断报告并等待用户介入：

```
❌ 已尝试3次，均未成功。

失败摘要：
- v2.4.7-fa.1031: update_tag job 失败 — softprops action 权限不足
- v2.4.7-fa.1032: 同上
- v2.4.7-fa.1033: 同上

建议检查：
1. repo Settings → Actions → Workflow permissions 是否为 Read and write
2. GITHUB_TOKEN 权限
```

---

## 版本号规则

### Fork 版本格式

`v{upstream_version}-fa.{increment}`（increment 为4位数）

- `v2.4.7-fa.1001` — 最初版本
- `v2.4.7-fa.1030` — 当前最新
- `v2.4.8-fa.1001` — 上游升级到 2.4.8 后的第一个版本

### 上游版本格式

`v{major}.{minor}.{patch}`，示例：`v2.4.7`

---

## Commit Message 规范

为了让 Changelog 自动分类正确，建议使用：

- `fix:` 或 `fix(scope):` → 🐞 修复问题
- `feat:` 或 `feat(scope):` → ✨ 新增功能
- `refactor:` 或 `perf:` → 🚀 优化改进

也支持中文关键词：包含 "修复" / "新增"、"添加" / "优化"、"改进"、"重构"

---

## 错误处理

### 问题：脚本执行失败

检查：

1. 是否有未提交的改动：`git status`
2. 是否安装了 jq：`which jq`
3. 权限：`ls -la scripts/prepare-release.sh`

### 问题：版本号已存在

使用 AskUserQuestion 工具询问：

```
版本号 v2.4.7-fa.1031 已存在。

请选择：
1. 自动递增到下一个版本号 (v2.4.7-fa.1032)
2. 自定义新的版本号
3. 强制覆盖现有版本（不推荐）
```

强制覆盖时：

```bash
git tag -d v2.4.7-fa.1031
git push origin :refs/tags/v2.4.7-fa.1031
./scripts/prepare-release.sh v2.4.7-fa.1031
```

### 问题：Changelog 分类不正确

- tag 推送前：直接编辑 Changelog.md，再重新运行 `prepare-release.sh`
- tag 已推送后：**不要 force push**，在下一个版本中补充说明

---

## 注意事项

1. **必须通过 `prepare-release.sh` 更新 Changelog.md 后再推送 tag**
   - `release.yml` 的 `check_tag_version` 会验证 Changelog.md 是否含有当前 tag 条目
   - 直接 `git tag` + `git push` 会被 CI 立即拦截

2. **确保在正确的分支上操作**（通常在 `dev` 或 `fa/v{version}` 分支）

3. **检查 GitHub Actions 权限**：需要 `contents: write`

---

## 相关文件

- `scripts/prepare-release.sh` — 发布脚本（Changelog生成 + 版本更新 + tag）
- `Changelog.md` — 更新日志
- `.github/workflows/release.yml` — 发布 workflow（含 Changelog 验证）
- `.github/workflows/updater.yml` — 更新器 workflow

---

## 示例对话

用户："帮我发布一个新版本"

助手执行：

1. 获取最新版本：`v2.4.7-fa.1030`，建议 `v2.4.7-fa.1031`
2. 询问用户确认
3. 运行 `./scripts/prepare-release.sh v2.4.7-fa.1031`
4. 推送成功，开始每分钟监控 GitHub Actions
5. 约22分钟后构建完成，验收 Release 内容
6. ✅ Release 正常：有 changelog、有产物、非草稿
