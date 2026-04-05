# 上游自动同步方案总结

## 📋 方案概述

**目标：** 自动监听上游 clash-verge-rev 仓库的新 release，并通过 AI 辅助的方式同步到你的 fork。

**实现方式：** GitHub Actions + Claude Code AI 辅助

**自动化程度：** 保守型（AI 辅助 + 人工审核）

## 🏗️ 架构设计

```
┌─────────────────────────────────────────────────────────────┐
│                    上游仓库发布新版本                          │
│           clash-verge-rev/clash-verge-rev                    │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│              GitHub Actions 定时检查                          │
│         每天 UTC 02:00 (北京时间 10:00)                       │
│              或手动触发                                        │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
              检测到新 release?
                     │
        ┌────────────┴────────────┐
        │                         │
       否                        是
        │                         │
        ▼                         ▼
     结束流程          创建测试分支 upstream-sync/v{version}
                                  │
                                  ▼
                          尝试自动合并上游代码
                                  │
                    ┌─────────────┴─────────────┐
                    │                           │
                 无冲突                       有冲突
                    │                           │
                    ▼                           ▼
            创建 PR (ready)              创建 PR (draft)
            标签: upstream-sync          标签: upstream-sync, conflicts
                    │                           │
                    ▼                           ▼
              运行 CI 测试              PR 中 @claude 请求帮助
                    │                           │
                    ▼                           ▼
            等待人工审核合并          Claude 分析并解决冲突
                                                │
                                                ▼
                                          运行 CI 测试
                                                │
                                                ▼
                                        人工审核冲突解决方案
                                                │
                                                ▼
                                          确认无误后合并
```

## 📁 创建的文件

### 1. `.github/workflows/upstream-sync.yml`

**核心工作流文件**

功能：

- 定时检查上游新 release（每天一次）
- 支持手动触发指定版本同步
- 自动创建测试分支
- 检测合并冲突
- 创建 PR 并根据情况 @claude

关键特性：

- 智能跳过已同步的版本
- 区分无冲突和有冲突的场景
- 自动添加合适的标签
- 包含上游 release notes

### 2. `.github/UPSTREAM_SYNC_GUIDE.md`

**用户使用指南**

内容：

- 工作原理说明
- 手动触发步骤
- 两种场景的处理流程
- 冲突解决策略
- 常见问题解答
- 最佳实践建议

### 3. `.github/CONFLICT_RESOLUTION_GUIDE.md`

**Claude AI 冲突解决指南**

内容：

- Fork 特定修改的识别规则
- 5 条冲突解决规则（优先级排序）
- 详细的解决工作流
- 特殊场景处理
- 测试检查清单
- 实际冲突解决示例

### 4. `.github/UPSTREAM_SYNC_TESTING.md`

**测试指南**

内容：

- 4 种测试场景
- 本地测试方法
- 验证清单
- 常见问题排查
- 监控和维护建议

### 5. Git Remote 配置

已添加 upstream remote：

```
upstream  https://github.com/clash-verge-rev/clash-verge-rev.git
```

## 🎯 核心特性

### 1. 智能冲突检测

- 自动识别是否有合并冲突
- 区分处理无冲突和有冲突的情况
- 提供详细的冲突文件列表

### 2. AI 辅助解决冲突

- 通过 @claude 提及触发 AI 帮助
- 提供详细的上下文和解决指令
- 遵循预定义的冲突解决规则

### 3. Fork 特定代码保护

- 识别 `// FORK:` 标记的代码
- 优先保留 fork 的自定义修改
- 智能合并上游改进

### 4. 完整的测试流程

- 在独立分支上进行合并
- 自动运行 CI 测试
- 人工审核后才合并到主分支

### 5. 灵活的触发方式

- 定时自动检查（每天一次）
- 手动触发指定版本
- 支持跳过特定版本

## 🔧 配置要求

### 必需配置

- ✅ Git upstream remote（已配置）
- ✅ GitHub Actions 权限（已在 workflow 中配置）
- ✅ 工作流文件（已创建）

### 可选配置

- ⚠️ Claude Code GitHub App（用于 AI 自动解决冲突）
  - 安装地址：https://github.com/apps/claude-code
  - 需要授予仓库访问权限

### 环境变量

在 workflow 中已配置：

```yaml
UPSTREAM_REPO: 'clash-verge-rev/clash-verge-rev'
UPSTREAM_OWNER: 'clash-verge-rev'
UPSTREAM_NAME: 'clash-verge-rev'
```

## 🚀 使用流程

### 首次使用

1. **提交配置文件**

   ```bash
   git add .github/workflows/upstream-sync.yml
   git add .github/UPSTREAM_SYNC_GUIDE.md
   git add .github/CONFLICT_RESOLUTION_GUIDE.md
   git add .github/UPSTREAM_SYNC_TESTING.md
   git commit -m "feat: add upstream auto-sync workflow with AI assistance"
   git push origin fa/v2.4.7-fa.0
   ```

2. **测试工作流**
   - 进入 GitHub Actions 页面
   - 手动触发 "Sync Upstream Release"
   - 输入测试版本号（如 `v2.4.7`）
   - 观察执行结果

3. **（可选）安装 Claude Code App**
   - 访问 https://github.com/apps/claude-code
   - 安装到你的仓库
   - 授予必要权限

### 日常使用

1. **自动模式**
   - 工作流每天自动检查
   - 发现新版本自动创建 PR
   - 收到通知后审核 PR

2. **手动模式**
   - 发现上游新版本
   - 手动触发工作流
   - 指定要同步的版本号

3. **审核和合并**
   - 查看 PR 中的变更
   - 等待 CI 测试通过
   - 如有冲突，审核 Claude 的解决方案
   - 确认无误后合并

## 📊 冲突解决策略

### 优先级规则

1. **最高优先级：保留 Fork 标记**
   - 所有 `// FORK:` 标记的代码必须保留
   - 上游改动不覆盖 fork 特定逻辑

2. **高优先级：接受上游 Bug 修复**
   - 安全补丁
   - Bug 修复
   - 性能优化

3. **中优先级：合并新功能**
   - 不冲突的新功能接受
   - 冲突的新功能需要适配

4. **中优先级：依赖更新**
   - 接受上游依赖版本升级
   - 验证兼容性

5. **询问优先级：不确定的情况**
   - 重大重构
   - 破坏性变更
   - 安全敏感代码

### Fork 特定修改示例

```yaml
# FORK: disabled, no Telegram channel
if: false

# FORK: disabled, no ARM Linux targets needed
if: false

# FORK: simplified release downloads
# Only show actually built platforms
```

## 🔍 监控和维护

### 定期检查

```bash
# 检查待处理的同步 PR
gh pr list --label upstream-sync --state open

# 检查工作流运行历史
gh run list --workflow=upstream-sync.yml --limit 5

# 检查上游新版本
git ls-remote --tags https://github.com/clash-verge-rev/clash-verge-rev.git | tail -5
```

### 通知设置

建议在 GitHub 设置中启用：

- Watch repository → Custom → Pull requests
- 每次创建同步 PR 时收到通知

## ⚠️ 注意事项

### 版本号差异

- **你的版本号格式：** v2.4.7.1017（带构建号）
- **上游版本号格式：** v2.4.7（标准语义化版本）
- 工作流会正确处理这种差异

### 测试建议

1. 首次使用建议手动触发测试
2. 选择一个已知稳定的版本测试
3. 验证 PR 创建和 CI 测试正常
4. 确认 Claude 响应正常（如已安装）

### 安全考虑

1. 所有操作在独立分支进行
2. 需要人工审核才能合并
3. CI 测试必须通过
4. 敏感代码冲突需要人工介入

## 📚 参考文档

### 内部文档

- `.github/UPSTREAM_SYNC_GUIDE.md` - 使用指南
- `.github/CONFLICT_RESOLUTION_GUIDE.md` - 冲突解决指南
- `.github/UPSTREAM_SYNC_TESTING.md` - 测试指南

### 外部资源

- [GitHub Actions 文档](https://docs.github.com/en/actions)
- [Claude Code GitHub Actions](https://docs.claude.com/en/docs/claude-code/github-actions)
- [Git 冲突解决](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/addressing-merge-conflicts)

## 🎉 优势总结

### 相比手动同步

- ✅ 自动检测新版本，不会错过更新
- ✅ 标准化的同步流程
- ✅ 完整的测试验证
- ✅ 详细的变更记录

### 相比完全自动化

- ✅ 人工审核保证质量
- ✅ 复杂冲突有人工介入
- ✅ 保留决策控制权
- ✅ 降低自动合并风险

### AI 辅助的价值

- ✅ 自动处理简单冲突
- ✅ 遵循预定义规则
- ✅ 节省人工时间
- ✅ 提供解决建议

## 🔮 未来扩展

### 可选增强

1. **通知集成**
   - Slack/Discord 通知
   - 邮件提醒
   - 企业微信/钉钉

2. **更智能的冲突解决**
   - 学习历史冲突解决模式
   - 自动识别常见冲突类型
   - 提供多个解决方案供选择

3. **自动化测试增强**
   - 运行完整的 E2E 测试
   - 性能回归测试
   - 安全扫描

4. **版本管理优化**
   - 自动更新 CHANGELOG
   - 生成版本对比报告
   - 追踪 fork 与上游的差异

## ✅ 下一步行动

1. **立即执行**
   - [ ] 提交所有配置文件
   - [ ] 推送到 GitHub
   - [ ] 手动触发测试工作流

2. **可选配置**
   - [ ] 安装 Claude Code GitHub App
   - [ ] 配置通知渠道
   - [ ] 调整 cron 时间（如需要）

3. **持续优化**
   - [ ] 根据实际使用调整规则
   - [ ] 完善冲突解决策略
   - [ ] 收集反馈改进流程

---

**创建时间：** 2026-04-06
**方案版本：** v1.0
**维护者：** 刘金发
