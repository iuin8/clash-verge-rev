# Upstream Sync - Testing Guide

本文档说明如何测试上游同步工作流。

## 前置准备

### 1. 配置 Git Remote

确认 upstream remote 已添加：

```bash
git remote -v
# 应该看到：
# upstream  https://github.com/clash-verge-rev/clash-verge-rev.git (fetch)
# upstream  https://github.com/clash-verge-rev/clash-verge-rev.git (push)
```

如果没有，运行：

```bash
git remote add upstream https://github.com/clash-verge-rev/clash-verge-rev.git
```

### 2. 安装 Claude Code GitHub App（可选）

如果要使用 AI 自动解决冲突：

1. 访问 https://github.com/apps/claude-code
2. 点击 "Install" 安装到你的仓库
3. 授予必要的权限（读写代码、PR、Issues）

## 测试场景

### 场景 1：手动触发同步（推荐首次测试）

这是最安全的测试方式，不会自动运行。

**步骤：**

1. 进入 GitHub Actions 页面：

   ```
   https://github.com/iuin8/clash-verge-rev/actions
   ```

2. 选择 "Sync Upstream Release" 工作流

3. 点击 "Run workflow" 按钮

4. 输入要测试的上游 tag（建议使用已知的稳定版本）：

   ```
   v2.4.7
   ```

5. 点击 "Run workflow" 确认

6. 观察工作流执行：
   - 检查是否成功创建了 `upstream-sync/v2.4.7` 分支
   - 检查是否创建了 PR
   - 如果有冲突，检查 PR 中是否正确 @claude

**预期结果：**

- ✅ 工作流成功运行
- ✅ 创建了新分支 `upstream-sync/v2.4.7`
- ✅ 创建了 PR（可能是 draft 如果有冲突）
- ✅ PR 中包含上游的 release notes
- ✅ 如果有冲突，PR 中有 @claude 的提及

### 场景 2：测试无冲突合并

选择一个你确定不会有冲突的旧版本测试。

**步骤：**

1. 查看上游历史版本：

   ```bash
   git ls-remote --tags --refs https://github.com/clash-verge-rev/clash-verge-rev.git | tail -10
   ```

2. 选择一个比你当前版本旧的 tag（例如 v2.4.3）

3. 手动触发工作流，输入 `v2.4.3`

4. 观察工作流：
   - 应该显示 "Merge successful without conflicts"
   - 创建的 PR 应该标记为 ready（非 draft）
   - PR 标签应该包含 `upstream-sync` 和 `automated`

**预期结果：**

- ✅ 合并成功，无冲突
- ✅ PR 可以直接审核和合并
- ✅ CI 测试自动运行

### 场景 3：测试冲突检测

选择最新的上游版本，很可能会有冲突。

**步骤：**

1. 手动触发工作流，输入 `v2.4.7`（上游最新版本）

2. 观察工作流：
   - 应该检测到冲突
   - 创建 draft PR
   - PR 中应该有冲突文件列表
   - PR 中应该有 @claude 的详细指令

3. 如果安装了 Claude Code App：
   - Claude 应该自动响应
   - Claude 会分析冲突并提交解决方案
   - 检查 Claude 的提交是否合理

**预期结果：**

- ✅ 正确检测到冲突
- ✅ 创建 draft PR
- ✅ PR 中列出冲突文件
- ✅ @claude 被正确提及
- ✅ Claude 响应并尝试解决（如果已安装 App）

### 场景 4：测试定时触发（可选）

这会在每天 UTC 02:00 自动运行。

**步骤：**

1. 等待定时任务自动触发（北京时间 10:00）

2. 或者修改 cron 表达式测试：

   ```yaml
   schedule:
     - cron: '*/5 * * * *' # 每 5 分钟运行一次（仅用于测试）
   ```

3. 提交修改后等待触发

4. 测试完成后记得改回原来的 cron 表达式

**注意：** 不建议长期使用高频 cron，会浪费 GitHub Actions 配额。

## 本地测试

在推送到 GitHub 之前，可以在本地测试合并逻辑。

### 测试合并流程

```bash
# 1. 获取上游最新代码
git fetch upstream --tags

# 2. 创建测试分支
git checkout -b test-upstream-sync

# 3. 尝试合并上游 tag
git merge upstream/v2.4.7

# 4. 如果有冲突，查看冲突文件
git status
git diff --name-only --diff-filter=U

# 5. 手动解决冲突（参考 CONFLICT_RESOLUTION_GUIDE.md）
# 编辑冲突文件...

# 6. 标记为已解决
git add .
git commit -m "test: resolve conflicts for upstream v2.4.7"

# 7. 验证构建
pnpm typecheck
pnpm lint
cargo clippy-all

# 8. 清理测试分支
git checkout fa/v2.4.7-fa.0
git branch -D test-upstream-sync
```

### 测试工作流语法

验证 workflow 文件语法正确：

```bash
# 使用 actionlint（需要先安装）
brew install actionlint  # macOS
# 或
go install github.com/rhysd/actionlint/cmd/actionlint@latest

# 检查语法
actionlint .github/workflows/upstream-sync.yml
```

## 验证清单

在正式启用自动同步之前，确认以下项目：

### 工作流配置

- [ ] `.github/workflows/upstream-sync.yml` 已创建
- [ ] 工作流语法正确（使用 actionlint 验证）
- [ ] upstream remote 已配置
- [ ] 环境变量正确（UPSTREAM_REPO 等）

### 权限配置

- [ ] 工作流有 `contents: write` 权限
- [ ] 工作流有 `pull-requests: write` 权限
- [ ] 工作流有 `issues: write` 权限
- [ ] GITHUB_TOKEN 可以创建 PR

### Claude Code 集成（可选）

- [ ] Claude Code GitHub App 已安装
- [ ] App 有访问仓库的权限
- [ ] 测试 @claude 提及是否触发响应

### 文档

- [ ] `UPSTREAM_SYNC_GUIDE.md` 已创建
- [ ] `CONFLICT_RESOLUTION_GUIDE.md` 已创建
- [ ] 团队成员了解同步流程

### 测试结果

- [ ] 手动触发测试成功
- [ ] 无冲突场景测试通过
- [ ] 冲突检测测试通过
- [ ] PR 创建成功
- [ ] CI 测试正常运行

## 常见问题排查

### 问题 1：工作流没有触发

**可能原因：**

- cron 表达式错误
- 工作流文件语法错误
- 仓库 Actions 被禁用

**排查步骤：**

```bash
# 检查 Actions 是否启用
# 访问：https://github.com/iuin8/clash-verge-rev/settings/actions

# 验证工作流语法
actionlint .github/workflows/upstream-sync.yml

# 手动触发测试
# 在 Actions 页面点击 "Run workflow"
```

### 问题 2：无法创建 PR

**可能原因：**

- GITHUB_TOKEN 权限不足
- 分支保护规则阻止
- 已存在同名 PR

**排查步骤：**

```bash
# 检查是否已存在同名分支
git ls-remote --heads origin | grep upstream-sync

# 检查是否已存在相关 PR
gh pr list --label upstream-sync

# 检查分支保护规则
# 访问：https://github.com/iuin8/clash-verge-rev/settings/branches
```

### 问题 3：Claude 没有响应

**可能原因：**

- Claude Code App 未安装
- App 权限不足
- @claude 提及格式错误

**排查步骤：**

```bash
# 检查 App 是否安装
# 访问：https://github.com/settings/installations

# 检查 PR 中的提及格式
# 应该是：@claude（不是 @claude-code 或其他）

# 手动在 PR 中评论测试
# 评论：@claude help
```

### 问题 4：合并冲突无法自动解决

**这是正常的！** 复杂冲突需要人工介入。

**处理步骤：**

1. 查看 Claude 的尝试（如果有）
2. 本地检出 PR 分支
3. 手动解决冲突
4. 参考 `CONFLICT_RESOLUTION_GUIDE.md`
5. 推送解决方案

## 监控和维护

### 定期检查

建议每周检查一次：

```bash
# 检查是否有待处理的同步 PR
gh pr list --label upstream-sync --state open

# 检查最近的工作流运行
gh run list --workflow=upstream-sync.yml --limit 5

# 检查上游是否有新版本
git ls-remote --tags https://github.com/clash-verge-rev/clash-verge-rev.git | tail -5
```

### 日志查看

如果工作流失败，查看详细日志：

```bash
# 列出最近的运行
gh run list --workflow=upstream-sync.yml

# 查看特定运行的日志
gh run view <run-id> --log
```

### 性能优化

如果工作流运行时间过长：

1. 减少 `fetch-depth`（如果不需要完整历史）
2. 使用缓存加速依赖安装
3. 并行运行独立的检查步骤

## 下一步

测试通过后：

1. ✅ 保持 cron 定时任务启用
2. ✅ 监控第一次自动触发的结果
3. ✅ 根据实际使用情况调整配置
4. ✅ 考虑添加通知（Slack、邮件等）
5. ✅ 定期审查和合并同步 PR

## 参考资源

- [GitHub Actions 调试指南](https://docs.github.com/en/actions/monitoring-and-troubleshooting-workflows)
- [gh CLI 文档](https://cli.github.com/manual/)
- [actionlint 文档](https://github.com/rhysd/actionlint)
