---
name: release
description: >-
  发布 clash-verge-rev 新版本 - 自动获取当前版本，建议新版本号，
  询问用户确认，运行发布脚本更新 Changelog.md，创建 tag 并推送，
  触发 GitHub Actions 构建。
  TRIGGER when: 用户说"发布"、"release"、"新版本"、"打tag"或请求发布操作。
  DO NOT TRIGGER when: 用户只是询问版本信息或查看 Changelog。
origin: project
---

# Release — 发布新版本

自动化 clash-verge-rev 的完整发布流程，从版本号确认到 GitHub Release 创建。

## 工作流程

1. 获取当前最新版本号
2. 建议下一个版本号
3. 询问用户确认或自定义版本号
4. 运行 `scripts/prepare-release.sh` 脚本
5. 自动更新 Changelog.md
6. 创建并推送 tag
7. 触发 GitHub Actions 构建

## 执行步骤

### 1. 获取当前版本信息

```bash
# 获取最新的 tag
git fetch --tags
LATEST_TAG=$(git tag --sort=-version:refname | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9]+\.[0-9]+)?$' | head -1)

# 获取上游版本号
UPSTREAM_VERSION=$(jq -r '.version' package.json)
```

### 2. 建议版本号

根据最新 tag 自动递增：

- 如果最新是 `v2.4.7-fa.0`，建议 `v2.4.7-fa.1`
- 如果最新是 `v2.4.7-fa.9`，建议 `v2.4.7-fa.10`

版本号格式：`v{upstream_version}-fa.{increment}`

### 3. 询问用户确认

使用 AskUserQuestion 工具询问：

```
当前最新版本: v2.4.7-fa.0
上游版本: v2.4.7
建议新版本: v2.4.7-fa.1

请选择：
1. 使用建议版本 v2.4.7-fa.1
2. 自定义版本号
```

如果用户选择自定义，再次询问具体版本号。

### 4. 执行发布脚本

```bash
./scripts/prepare-release.sh v2.4.7-fa.1
```

脚本会：

- 分析自上次 tag 以来的所有 commit messages
- 自动分类（修复/新增/优化）
- 生成 Changelog.md 条目
- 显示生成的内容供用户确认
- 提交 Changelog.md
- 创建 tag
- 推送到远程仓库

### 5. 验证发布

```bash
# 检查 tag 是否创建成功
git tag | grep v2.4.7-fa.1

# 检查 Changelog.md 是否更新
head -20 Changelog.md

# 检查 GitHub Actions 是否触发
echo "请访问 https://github.com/你的用户名/clash-verge-rev/actions 查看构建状态"
```

## 版本号规则

### Fork 版本格式

`v{upstream_version}-fa.{increment}`

示例：

- `v2.4.7-fa.0` - 基于上游 2.4.7 的第一个 fork 版本
- `v2.4.7-fa.1` - 基于上游 2.4.7 的第二个 fork 版本
- `v2.4.8-fa.0` - 上游升级到 2.4.8 后的第一个 fork 版本

### 上游版本格式

`v{major}.{minor}.{patch}`

示例：`v2.4.7`

## Commit Message 规范

为了让 Changelog 自动分类正确，建议使用：

- `fix:` 或 `fix(scope):` - 修复问题
- `feat:` 或 `feat(scope):` - 新增功能
- `refactor:` 或 `perf:` - 优化改进

也支持中文关键词：

- 包含 "修复" → 🐞 修复问题
- 包含 "新增"、"添加" → ✨ 新增功能
- 包含 "优化"、"改进"、"重构" → 🚀 优化改进

## 错误处理

### 问题：脚本执行失败

检查：

1. 是否有未提交的改动：`git status`
2. 是否有权限问题：`ls -la scripts/prepare-release.sh`
3. 是否安装了 jq：`which jq`

### 问题：版本号已存在

**不要直接删除 tag！** 应该询问用户：

使用 AskUserQuestion 工具询问：

```
版本号 v2.4.7-fa.1 已存在。

请选择：
1. 自动递增到下一个版本号 (v2.4.7-fa.2)
2. 自定义新的版本号
3. 强制覆盖现有版本（不推荐，仅用于修复错误）
```

如果用户选择强制覆盖，再次确认并执行：

```bash
# 删除本地 tag
git tag -d v2.4.7-fa.1

# 删除远程 tag（如果已推送）
git push origin :refs/tags/v2.4.7-fa.1

# 重新运行脚本
./scripts/prepare-release.sh v2.4.7-fa.1
```

### 问题：Changelog 分类不正确

手动编辑 Changelog.md 调整分类，然后：

```bash
git add Changelog.md
git commit --amend --no-edit
git tag -f v2.4.7-fa.1
git push origin HEAD --force-with-lease
git push origin v2.4.7-fa.1 --force
```

## 注意事项

1. **必须在推送 tag 之前更新 Changelog.md**
   - release.yml 读取的是 tag 时刻的代码
   - 脚本已经处理了这个顺序

2. **确保在正确的分支上操作**
   - 通常在 `dev` 或 `fa/v{version}` 分支

3. **检查 GitHub Actions 权限**
   - 需要 `contents: write` 权限

4. **验证构建状态**
   - 推送后访问 GitHub Actions 页面查看构建进度

## 相关文件

- `scripts/prepare-release.sh` - 发布脚本
- `docs/RELEASE.md` - 详细发布文档
- `Changelog.md` - 更新日志
- `.github/workflows/release.yml` - 发布 workflow
- `.github/workflows/updater.yml` - 更新器 workflow

## 示例对话

用户："帮我发布一个新版本"

助手执行：

1. 获取最新版本：`v2.4.7-fa.0`
2. 建议新版本：`v2.4.7-fa.1`
3. 询问用户确认
4. 运行 `./scripts/prepare-release.sh v2.4.7-fa.1`
5. 验证发布成功
