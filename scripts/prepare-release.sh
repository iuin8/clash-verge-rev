#!/bin/bash

# 准备发布：自动更新 Changelog.md 并创建 tag
# 用法: ./scripts/prepare-release.sh v2.4.7.1002 [--yes]
#
# 选项:
#   --yes, -y    非交互模式，自动确认所有操作

set -e

# 清理临时文件
TEMP_CHANGELOG=""
TEMP_FILE=""
trap 'rm -f "$TEMP_CHANGELOG" "$TEMP_FILE"' EXIT

# 解析参数
AUTO_YES=false
TAG_NAME=""

for arg in "$@"; do
  case $arg in
    --yes|-y)
      AUTO_YES=true
      shift
      ;;
    *)
      if [ -z "$TAG_NAME" ]; then
        TAG_NAME="$arg"
      fi
      shift
      ;;
  esac
done

if [ -z "$TAG_NAME" ]; then
  echo "错误: 请提供版本号"
  echo "用法: $0 v2.4.7.1002 [--yes]"
  exit 1
fi

VERSION="${TAG_NAME#v}"

# 验证版本号格式
if ! echo "$TAG_NAME" | grep -qE '^v[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9]+\.[0-9]+)?$'; then
  echo "错误: 版本号格式不正确，应为 vX.Y.Z 或 vX.Y.Z-suffix.N"
  echo "示例: v2.4.7 或 v2.4.7-fa.0"
  exit 1
fi

echo "准备发布版本: $TAG_NAME"

# 检查工作区状态
if ! git diff-index --quiet HEAD --; then
  echo "错误: 工作区有未提交的改动，请先提交或暂存"
  echo ""
  git status --short
  exit 1
fi

# 检查依赖
if ! command -v jq &> /dev/null; then
  echo "警告: 未安装 jq，无法获取上游版本号"
  echo "建议安装: brew install jq (macOS) 或 apt install jq (Linux)"
fi

# 获取上一个 tag
PREVIOUS_TAG=$(git tag --sort=-version:refname | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9]+\.[0-9]+)?$' | head -1)

if [ -z "$PREVIOUS_TAG" ]; then
  echo "警告: 未找到上一个 tag，将使用所有提交"
  PREVIOUS_TAG=$(git rev-list --max-parents=0 HEAD)
else
  echo "上一个版本: $PREVIOUS_TAG"
fi

# 获取两个 tag 之间的 commit messages
echo "正在分析提交记录..."
COMMITS=$(git log --pretty=format:"%s" ${PREVIOUS_TAG}..HEAD)

# 分类 commit messages
FIXES=""
FEATURES=""
IMPROVEMENTS=""
OTHERS=""

while IFS= read -r commit; do
  # 跳过空行
  [ -z "$commit" ] && continue

  case "$commit" in
    fix:*|fix\(*|修复:*|修复\(*)
      FIXES="${FIXES}- ${commit#*: }\n"
      ;;
    feat:*|feat\(*|新增:*|新增\(*|feature:*|feature\(*)
      FEATURES="${FEATURES}- ${commit#*: }\n"
      ;;
    refactor:*|refactor\(*|perf:*|perf\(*|优化:*|优化\(*|重构:*|重构\(*)
      IMPROVEMENTS="${IMPROVEMENTS}- ${commit#*: }\n"
      ;;
    *)
      # 如果 commit message 包含中文关键词，也进行分类
      if echo "$commit" | grep -qE "修复|fix"; then
        FIXES="${FIXES}- ${commit}\n"
      elif echo "$commit" | grep -qE "新增|添加|feat|feature"; then
        FEATURES="${FEATURES}- ${commit}\n"
      elif echo "$commit" | grep -qE "优化|改进|重构|refactor|perf"; then
        IMPROVEMENTS="${IMPROVEMENTS}- ${commit}\n"
      else
        OTHERS="${OTHERS}- ${commit}\n"
      fi
      ;;
  esac
done <<< "$COMMITS"

# 生成 changelog 内容
CHANGELOG_CONTENT="## v${VERSION}\n\n"

# 判断是否是 fork 版本（包含 -suffix.N 格式）
if echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+-[a-zA-Z0-9]+\.[0-9]+$'; then
  # 从 package.json 获取上游版本号
  UPSTREAM_VERSION=$(jq -r '.version' package.json 2>/dev/null || echo "unknown")

  CHANGELOG_CONTENT="${CHANGELOG_CONTENT}> [!IMPORTANT]\n"
  if [ "$UPSTREAM_VERSION" != "unknown" ]; then
    CHANGELOG_CONTENT="${CHANGELOG_CONTENT}> 这是基于上游 v${UPSTREAM_VERSION} 的个人 fork 版本。\n\n"
  else
    CHANGELOG_CONTENT="${CHANGELOG_CONTENT}> 这是基于上游的个人 fork 版本。\n\n"
  fi
fi

if [ -n "$FIXES" ]; then
  CHANGELOG_CONTENT="${CHANGELOG_CONTENT}### 🐞 修复问题\n\n${FIXES}\n"
fi

if [ -n "$FEATURES" ]; then
  CHANGELOG_CONTENT="${CHANGELOG_CONTENT}### ✨ 新增功能\n\n${FEATURES}\n"
fi

if [ -n "$IMPROVEMENTS" ]; then
  CHANGELOG_CONTENT="${CHANGELOG_CONTENT}### 🚀 优化改进\n\n${IMPROVEMENTS}\n"
fi

if [ -n "$OTHERS" ] && [ -z "$FIXES" ] && [ -z "$FEATURES" ] && [ -z "$IMPROVEMENTS" ]; then
  CHANGELOG_CONTENT="${CHANGELOG_CONTENT}### 📝 更新内容\n\n${OTHERS}\n"
fi

CHANGELOG_CONTENT="${CHANGELOG_CONTENT}---\n\n"

# 保存到临时文件（使用 mktemp 确保安全）
TEMP_CHANGELOG=$(mktemp)
echo -e "$CHANGELOG_CONTENT" > "$TEMP_CHANGELOG"

echo ""
echo "生成的 Changelog 内容:"
echo "================================"
cat "$TEMP_CHANGELOG"
echo "================================"
echo ""

# 检查是否已经存在该版本的 changelog
if [ -f "Changelog.md" ] && grep -q "^## v${VERSION}" Changelog.md; then
  echo "警告: Changelog.md 中已存在版本 v${VERSION}"

  if [ "$AUTO_YES" = true ]; then
    echo "非交互模式: 自动覆盖现有版本"
  else
    read -p "是否覆盖? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
      echo "已取消"
      exit 1
    fi
  fi

  # 删除旧的版本条目（跨平台兼容）
  if [[ "$OSTYPE" == "darwin"* ]]; then
    # macOS (BSD sed)
    sed -i '' "/^## v${VERSION}/,/^---$/d" Changelog.md
  else
    # Linux (GNU sed)
    sed -i "/^## v${VERSION}/,/^---$/d" Changelog.md
  fi
fi

# 更新 Changelog.md
if [ ! -f "Changelog.md" ]; then
  echo "# Changelog" > Changelog.md
  echo "" >> Changelog.md
fi

# 创建临时文件
TEMP_FILE=$(mktemp)

# 将新内容插入到文件开头（在 # Changelog 之后）
if grep -q "^# Changelog" Changelog.md; then
  # 提取 # Changelog 行
  head -n 1 Changelog.md > "$TEMP_FILE"
  echo "" >> "$TEMP_FILE"
  # 添加新内容
  cat "$TEMP_CHANGELOG" >> "$TEMP_FILE"
  # 添加剩余内容（跳过第一行和第一个空行）
  tail -n +2 Changelog.md | sed '1{/^$/d}' >> "$TEMP_FILE"
else
  # 否则直接插入到文件开头
  cat "$TEMP_CHANGELOG" > "$TEMP_FILE"
  cat Changelog.md >> "$TEMP_FILE"
fi

mv "$TEMP_FILE" Changelog.md

echo "✅ Changelog.md 已更新"
echo ""

# 询问是否提交
if [ "$AUTO_YES" = true ]; then
  echo "非交互模式: 自动提交并创建 tag"
  SHOULD_COMMIT=true
else
  read -p "是否提交 Changelog.md 并创建 tag? (Y/n) " -n 1 -r
  echo
  if [[ $REPLY =~ ^[Nn]$ ]]; then
    echo "已取消。Changelog.md 已更新但未提交。"
    exit 0
  fi
  SHOULD_COMMIT=true
fi

if [ "$SHOULD_COMMIT" = true ]; then
  # 提交 Changelog.md
  git add Changelog.md
  git commit -m "docs: 更新 Changelog.md for ${TAG_NAME}"

  echo "✅ 已提交 Changelog.md"
  echo ""

  # 创建 tag
  git tag "$TAG_NAME"
  echo "✅ 已创建 tag: $TAG_NAME"
  echo ""
fi

# 询问是否推送
if [ "$AUTO_YES" = true ]; then
  echo "非交互模式: 自动推送到远程仓库"
  SHOULD_PUSH=true
else
  read -p "是否推送到远程仓库? (Y/n) " -n 1 -r
  echo
  if [[ $REPLY =~ ^[Nn]$ ]]; then
    echo "已取消推送。请手动执行:"
    echo "  git push origin HEAD"
    echo "  git push origin $TAG_NAME"
    exit 0
  fi
  SHOULD_PUSH=true
fi

if [ "$SHOULD_PUSH" = true ]; then
  # 推送
  CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
  git push origin "$CURRENT_BRANCH"
  git push origin "$TAG_NAME"

  echo ""
  echo "🎉 发布准备完成！"
  echo "   - Changelog.md 已更新并提交"
  echo "   - Tag $TAG_NAME 已创建并推送"
  echo "   - GitHub Actions 将自动开始构建发布"
fi
