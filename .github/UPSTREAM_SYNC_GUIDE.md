# Upstream Sync Guide

本文档说明如何使用自动化工作流同步上游 clash-verge-rev 仓库的更新。

## 工作原理

### 自动触发

工作流每天自动检查上游是否有新的 release：

- 每天 UTC 时间 02:00（北京时间 10:00）自动运行
- 检测到新的 release tag 时自动创建同步 PR

### 手动触发

你也可以手动触发同步特定版本：

1. 进入 Actions 页面
2. 选择 "Sync Upstream Release" 工作流
3. 点击 "Run workflow"
4. 输入要同步的上游 tag（例如：`v1.7.7`）
5. 点击 "Run workflow" 确认

## 同步流程

### 场景 1：无冲突的干净合并

```
检测到新 release (v1.7.7)
    ↓
创建分支: upstream-sync/v1.7.7
    ↓
自动合并成功 ✅
    ↓
创建 PR（标签：upstream-sync, automated）
    ↓
CI 测试自动运行
    ↓
等待人工审核和合并
```

**你需要做的：**

1. 查看 PR 中的变更
2. 等待 CI 测试通过
3. 确认没有问题后合并 PR

### 场景 2：有冲突需要解决

```
检测到新 release (v1.7.7)
    ↓
创建分支: upstream-sync/v1.7.7
    ↓
检测到合并冲突 ⚠️
    ↓
创建 Draft PR（标签：upstream-sync, conflicts, automated）
    ↓
PR 中自动 @claude 请求帮助
    ↓
Claude 分析冲突并提交解决方案
    ↓
CI 测试运行
    ↓
等待人工审核冲突解决方案
    ↓
确认无误后合并 PR
```

**你需要做的：**

1. 查看 PR，Claude 会自动被 @ 提及
2. Claude 会分析冲突并提交解决方案
3. 审查 Claude 的冲突解决方案：
   - 检查 `// FORK:` 标记的自定义代码是否被保留
   - 验证上游的重要修复是否被正确合并
4. 如果 Claude 的解决方案有问题，在 PR 中评论指导 Claude 修正
5. CI 测试通过后，合并 PR

## 冲突解决策略

### Fork 特定代码的标记

我们使用 `// FORK:` 注释标记 fork 特定的修改：

```typescript
// FORK: disabled, no Telegram channel
if: false
```

```rust
// FORK: custom modification for personal use
let custom_config = load_custom_config();
```

### Claude 的冲突解决原则

Claude 在解决冲突时会遵循以下原则：

1. **保留 fork 特定修改**
   - 所有带 `// FORK:` 标记的代码优先保留
   - 自定义配置和功能不被上游覆盖

2. **接受上游的重要更新**
   - Bug 修复
   - 安全补丁
   - 性能优化
   - 新功能（不与 fork 修改冲突的）

3. **智能合并**
   - 如果上游修改了我们也修改过的文件，Claude 会尝试合并双方的改动
   - 对于不确定的情况，Claude 会在 PR 中提问

## 配置说明

### 环境变量

在 `.github/workflows/upstream-sync.yml` 中配置：

```yaml
env:
  UPSTREAM_REPO: 'clash-verge-rev/clash-verge-rev' # 上游仓库
  UPSTREAM_OWNER: 'clash-verge-rev' # 上游所有者
  UPSTREAM_NAME: 'clash-verge-rev' # 上游仓库名
```

### 所需权限

工作流需要以下权限（已在 workflow 中配置）：

- `contents: write` - 创建分支和提交
- `pull-requests: write` - 创建和更新 PR
- `issues: write` - 在 PR 中评论

### Claude Code Action

要启用 Claude 自动解决冲突，需要：

1. 在仓库中安装 Claude Code GitHub App
2. 配置 `ANTHROPIC_API_KEY` secret（如果使用 API 方式）
3. 在 PR 中 @claude 即可触发

## 常见问题

### Q: 如何跳过某个上游版本？

A: 如果某个上游版本不想同步，可以：

1. 关闭对应的 PR
2. 手动在本地创建一个同名的 tag：`git tag v1.7.7 && git push origin v1.7.7`
3. 这样工作流会认为该版本已同步，不会再创建 PR

### Q: 如果 Claude 解决冲突失败怎么办？

A: 可以手动解决：

1. 检出 PR 的分支：`git fetch origin upstream-sync/v1.7.7 && git checkout upstream-sync/v1.7.7`
2. 手动解决冲突：`git merge upstream/v1.7.7`
3. 解决冲突后提交：`git add . && git commit`
4. 推送：`git push origin upstream-sync/v1.7.7`

### Q: 如何测试同步后的代码？

A: 在合并 PR 之前：

1. 检出 PR 分支到本地
2. 运行完整的测试套件：
   ```bash
   pnpm install
   pnpm typecheck
   pnpm lint
   cargo clippy-all
   cargo test
   pnpm dev  # 本地测试运行
   ```
3. 确认功能正常后再合并

### Q: 同步频率可以调整吗？

A: 可以修改 `.github/workflows/upstream-sync.yml` 中的 cron 表达式：

```yaml
schedule:
  - cron: '0 2 * * *' # 每天一次
  # - cron: '0 2 * * 1'  # 每周一次
  # - cron: '0 2 1 * *'  # 每月一次
```

## 监控和通知

### PR 标签

- `upstream-sync` - 所有上游同步 PR
- `automated` - 自动创建的 PR
- `conflicts` - 有冲突需要解决的 PR

### 通知设置

建议在 GitHub 设置中启用以下通知：

- Watch this repository → Custom → Pull requests
- 这样每次创建同步 PR 时你都会收到通知

## 最佳实践

1. **定期检查 PR**
   - 即使是自动创建的 PR，也要人工审核
   - 特别关注有冲突的 PR

2. **保持 fork 修改最小化**
   - 尽量减少对上游代码的修改
   - 使用 `// FORK:` 清晰标记所有自定义修改
   - 考虑将自定义功能提交回上游

3. **测试后再合并**
   - 即使 CI 通过，也建议本地测试关键功能
   - 特别是有冲突解决的 PR

4. **记录重要决策**
   - 如果某个上游更新被有意跳过，在 PR 中记录原因
   - 方便未来回溯

## 参考资源

- [GitHub Actions 文档](https://docs.github.com/en/actions)
- [Claude Code GitHub Actions](https://docs.claude.com/en/docs/claude-code/github-actions)
- [Git 冲突解决指南](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/addressing-merge-conflicts)
