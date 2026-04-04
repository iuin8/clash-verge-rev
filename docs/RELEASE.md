# 发布流程说明

## 自动更新 Changelog.md 并发布

使用 `scripts/prepare-release.sh` 脚本可以自动完成以下操作：

1. 分析自上次 tag 以来的所有 commit messages
2. 自动分类并生成 Changelog.md 条目
3. 提交 Changelog.md
4. 创建并推送 tag
5. 触发 GitHub Actions 自动构建发布

### 使用方法

```bash
# 基本用法
./scripts/prepare-release.sh v2.4.7-fa.0

# 脚本会：
# 1. 显示生成的 Changelog 内容供你确认
# 2. 询问是否提交
# 3. 询问是否推送
```

### Commit Message 规范

为了让脚本正确分类，建议使用以下格式：

```bash
# 修复问题
git commit -m "fix: 修复更新检查问题"
git commit -m "fix(updater): 修复版本号显示"

# 新增功能
git commit -m "feat: 添加新功能"
git commit -m "feat(profiles): 新增拖拽排序"

# 优化改进
git commit -m "refactor: 重构代码结构"
git commit -m "perf: 优化性能"
```

脚本也支持中文关键词识别：

- 包含 "修复" 或 "fix" → 🐞 修复问题
- 包含 "新增"、"添加" 或 "feat" → ✨ 新增功能
- 包含 "优化"、"改进"、"重构" 或 "refactor" → 🚀 优化改进

### 完整发布流程

```bash
# 1. 确保所有改动已提交
git status

# 2. 运行发布脚本
./scripts/prepare-release.sh v2.4.7-fa.1

# 3. 脚本会显示生成的 Changelog，确认后：
#    - 自动提交 Changelog.md
#    - 创建 tag
#    - 推送到远程仓库

# 4. GitHub Actions 会自动：
#    - 构建所有平台的安装包
#    - 创建 GitHub Release
#    - 从 Changelog.md 提取更新说明
#    - 上传构建产物
```

### 手动流程（不推荐）

如果你想手动操作：

```bash
# 1. 手动编辑 Changelog.md，在顶部添加新版本条目

# 2. 提交 Changelog.md
git add Changelog.md
git commit -m "docs: 更新 Changelog.md for v2.4.7-fa.1"

# 3. 创建并推送 tag
git tag v2.4.7-fa.1
git push origin HEAD
git push origin v2.4.7-fa.1
```

### 注意事项

1. **版本号格式**：
   - Fork 版本：`v2.4.7-fa.0`（带 -suffix.N 后缀）
   - 上游版本：`v2.4.7`（标准三位版本号）

2. **分支**：
   - 通常在 `dev` 分支上操作
   - 脚本会自动推送到当前分支

3. **Tag 时机**：
   - 必须在推送 tag **之前**更新 Changelog.md
   - 因为 release.yml 读取的是 tag 时刻的代码

4. **重复版本**：
   - 如果 Changelog.md 中已存在该版本，脚本会询问是否覆盖

### 故障排查

**问题：Release 页面没有显示更新内容**

原因：Changelog.md 在 tag 之后才更新

解决：

```bash
# 删除错误的 tag
git tag -d v2.4.7-fa.1
git push origin :refs/tags/v2.4.7-fa.1

# 重新运行脚本
./scripts/prepare-release.sh v2.4.7-fa.1
```

**问题：生成的 Changelog 分类不正确**

原因：Commit message 格式不规范

解决：使用规范的 commit message 格式，或手动编辑 Changelog.md
